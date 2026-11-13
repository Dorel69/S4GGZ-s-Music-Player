use crate::settings::EffectDensity;
use eframe::egui::{self, Color32, Pos2, TextureHandle, Vec2};
use std::path::PathBuf;

const BASE_SCALE: f32 = 0.4;
const FISH_COUNT: usize = 5;
const ROUTE_LANES: [f32; 3] = [0.62, 0.75, 0.88];
const MOUSE_IDLE_SECONDS: f64 = 60.0;
const CLICK_BOOST_SECONDS: f32 = 1.5;
const HIT_SLOP: f32 = 2.0;

struct FishAnimationState {
    position: Vec2,
    horizontal_direction: f32,
    horizontal_speed: f32,
    vertical_velocity: f32,
    target_vertical_velocity: f32,
    vertical_drift: f32,
    change_timer: f32,
    route_lane: usize,
    route_target_y: f32,
    time_counter: f32,
    wave_angle: f32,
    scale_pulse: f32,
    facing_left: bool,
    texture_index: usize,
    orbiting: bool,
    orbit_phase: f32,
    orbit_radius: f32,
    orbit_speed: f32,
    fast_remaining: f32,
    random: u64,
    initialized: bool,
}

impl FishAnimationState {
    fn new(seed: u64) -> Self {
        Self {
            position: Vec2::ZERO,
            horizontal_direction: 1.0,
            horizontal_speed: 2.0,
            vertical_velocity: 0.0,
            target_vertical_velocity: 0.0,
            vertical_drift: 0.0,
            change_timer: 0.0,
            route_lane: 0,
            route_target_y: 0.0,
            time_counter: 0.0,
            wave_angle: 0.0,
            scale_pulse: BASE_SCALE,
            facing_left: false,
            texture_index: 0,
            orbiting: false,
            orbit_phase: 0.0,
            orbit_radius: 0.0,
            orbit_speed: 0.0,
            fast_remaining: 0.0,
            random: seed.max(1),
            initialized: false,
        }
    }

    fn random_unit(&mut self) -> f32 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        (self.random >> 40) as f32 / 16_777_215.0
    }

    fn random_between(&mut self, minimum: f32, maximum: f32) -> f32 {
        minimum + self.random_unit() * (maximum - minimum)
    }

    fn random_index(&mut self, count: usize) -> usize {
        ((self.random_unit() * count as f32) as usize).min(count.saturating_sub(1))
    }

    fn lower_vertical_bounds(bounds: Vec2, fish_size: Vec2) -> (f32, f32) {
        let half_height = fish_size.y * 0.225;
        let minimum = (bounds.y * 0.55 + half_height).min(bounds.y * 0.75);
        let maximum = (bounds.y * 0.94 - half_height).max(minimum);
        (minimum, maximum)
    }

    fn choose_route_lane(&mut self, bounds: Vec2, vertical_bounds: (f32, f32)) {
        let lane_count = ROUTE_LANES.len();
        let mut next_lane = (self.random_unit() * lane_count as f32) as usize;
        if next_lane >= lane_count {
            next_lane = lane_count - 1;
        }
        if next_lane == self.route_lane {
            next_lane = (next_lane + 1 + (self.random_unit() * (lane_count - 1) as f32) as usize)
                % lane_count;
        }
        self.route_lane = next_lane;
        let jitter = self.random_between(-0.035, 0.035) * bounds.y;
        self.route_target_y = (ROUTE_LANES[next_lane] * bounds.y + jitter)
            .clamp(vertical_bounds.0, vertical_bounds.1);
    }

    fn advance(&mut self, elapsed: f32, bounds: Vec2, fish_size: Vec2) {
        if self.orbiting {
            self.orbiting = false;
            self.route_target_y = self.position.y;
        }
        let vertical_bounds = Self::lower_vertical_bounds(bounds, fish_size);
        if !self.initialized {
            let horizontal_margin = 100.0_f32.min(bounds.x * 0.5);
            self.position = Vec2::new(
                self.random_between(
                    horizontal_margin,
                    (bounds.x - horizontal_margin).max(horizontal_margin),
                ),
                self.random_between(vertical_bounds.0, vertical_bounds.1),
            );
            self.horizontal_direction = if self.random_unit() < 0.5 { -1.0 } else { 1.0 };
            self.horizontal_speed = self.random_between(1.5, 3.0);
            self.vertical_velocity = self.random_between(-1.0, 1.0);
            self.route_lane = (self.random_unit() * ROUTE_LANES.len() as f32) as usize;
            self.route_lane = self.route_lane.min(ROUTE_LANES.len() - 1);
            self.route_target_y = (ROUTE_LANES[self.route_lane] * bounds.y)
                .clamp(vertical_bounds.0, vertical_bounds.1);
            self.initialized = true;
        }

        let elapsed = elapsed.clamp(0.0, 0.05);
        let frame_scale = elapsed * 60.0;
        let speed_multiplier = if self.fast_remaining > 0.0 { 2.8 } else { 1.0 };
        self.fast_remaining = (self.fast_remaining - elapsed).max(0.0);
        self.change_timer += elapsed;
        if self.change_timer > 2.0 {
            self.vertical_drift = self.random_between(-0.2, 0.2);
            self.change_timer = 0.0;
        }

        let depth_pull =
            ((self.route_target_y - self.position.y) / bounds.y.max(1.0) * 2.0).clamp(-0.8, 0.8);
        self.target_vertical_velocity = (depth_pull + self.vertical_drift).clamp(-1.2, 1.2);
        let smoothing = 1.0 - 0.95_f32.powf(frame_scale);
        self.vertical_velocity +=
            (self.target_vertical_velocity - self.vertical_velocity) * smoothing;
        self.position.x +=
            self.horizontal_direction * self.horizontal_speed * frame_scale * speed_multiplier;
        self.position.y += self.vertical_velocity * frame_scale;

        let horizontal_margin = 100.0_f32.min(bounds.x * 0.5);
        if self.position.x > bounds.x - horizontal_margin {
            self.position.x = bounds.x - horizontal_margin;
            self.horizontal_direction = -1.0;
            self.choose_route_lane(bounds, vertical_bounds);
        } else if self.position.x < horizontal_margin {
            self.position.x = horizontal_margin;
            self.horizontal_direction = 1.0;
            self.choose_route_lane(bounds, vertical_bounds);
        }

        if self.position.y < vertical_bounds.0 {
            self.position.y = vertical_bounds.0;
            self.vertical_velocity = self.vertical_velocity.abs();
        } else if self.position.y > vertical_bounds.1 {
            self.position.y = vertical_bounds.1;
            self.vertical_velocity = -self.vertical_velocity.abs();
        }

        self.time_counter += elapsed * 5.0;
        self.wave_angle = (self.time_counter * 1.5).sin() * 8.0;
        self.scale_pulse = BASE_SCALE + (self.time_counter * 0.8).sin() * 0.05;
        self.facing_left = self.horizontal_direction < 0.0;
    }

    fn advance_orbit(&mut self, elapsed: f32, center: Vec2, bounds: Vec2, fish_size: Vec2) {
        self.orbiting = true;
        self.orbit_phase = (self.orbit_phase + elapsed.clamp(0.0, 0.05) * self.orbit_speed)
            .rem_euclid(std::f32::consts::TAU);
        let offset = Vec2::angled(self.orbit_phase) * self.orbit_radius;
        self.position = center + offset;
        self.horizontal_direction = if -offset.y * self.orbit_speed < 0.0 {
            -1.0
        } else {
            1.0
        };
        self.facing_left = self.horizontal_direction < 0.0;
        let vertical_bounds = Self::lower_vertical_bounds(bounds, fish_size);
        self.position.y = self.position.y.clamp(vertical_bounds.0, vertical_bounds.1);
        self.time_counter += elapsed.clamp(0.0, 0.05) * 5.0;
        self.wave_angle = (self.time_counter * 1.5).sin() * 8.0;
        self.scale_pulse = BASE_SCALE + (self.time_counter * 0.8).sin() * 0.05;
    }
}

struct FishBubble {
    origin: Pos2,
    started_at: f64,
    lifetime: f32,
    horizontal_velocity: f32,
    rise_speed: f32,
    radius: f32,
}

impl FishBubble {
    fn age(&self, now: f64) -> f32 {
        (now - self.started_at).clamp(0.0, self.lifetime as f64) as f32
    }

    fn position(&self, now: f64) -> Pos2 {
        let age = self.age(now);
        let kick_duration = self.lifetime * 0.5;
        let kick_time = age.min(kick_duration);
        let rise_time = (age - kick_duration).max(0.0);
        Pos2::new(
            self.origin.x + self.horizontal_velocity * kick_time,
            self.origin.y - self.rise_speed * rise_time,
        )
    }

    fn opacity(&self, now: f64) -> f32 {
        ((self.lifetime - self.age(now)) / 0.5).clamp(0.0, 1.0)
    }

    fn paint(&self, painter: &egui::Painter, now: f64) {
        let opacity = self.opacity(now);
        if opacity <= 0.0 {
            return;
        }
        let center = self.position(now);
        let alpha = (opacity * 210.0).round() as u8;
        painter.circle_filled(
            center,
            self.radius,
            Color32::from_rgba_unmultiplied(150, 224, 255, alpha / 3),
        );
        painter.circle_stroke(
            center,
            self.radius,
            egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(230, 250, 255, alpha)),
        );
    }
}

struct AlphaMask {
    width: usize,
    height: usize,
    alpha: Vec<u8>,
}

impl AlphaMask {
    fn contains_opaque_pixel(
        &self,
        point: Pos2,
        center: Pos2,
        size: Vec2,
        angle: f32,
        flip_horizontal: bool,
    ) -> bool {
        if self.width == 0 || self.height == 0 || size.x <= 0.0 || size.y <= 0.0 {
            return false;
        }
        let relative = point - center;
        let (sin, cos) = angle.sin_cos();
        let local = Vec2::new(
            relative.x * cos + relative.y * sin,
            -relative.x * sin + relative.y * cos,
        );
        let u = local.x / size.x + 0.5;
        let v = local.y / size.y + 0.5;
        let texture_u = if flip_horizontal { 1.0 - u } else { u };
        // Match the filtered texture, with a small screen-space tolerance for thin fins.
        let x = texture_u * self.width as f32 - 0.5;
        let y = v * self.height as f32 - 0.5;
        let radius_x = HIT_SLOP * self.width as f32 / size.x + 1.0;
        let radius_y = HIT_SLOP * self.height as f32 / size.y + 1.0;
        let min_x = (x - radius_x).ceil().max(0.0) as usize;
        let min_y = (y - radius_y).ceil().max(0.0) as usize;
        let max_x = (x + radius_x).floor().min(self.width as f32 - 1.0) as isize;
        let max_y = (y + radius_y).floor().min(self.height as f32 - 1.0) as isize;
        if max_x < min_x as isize || max_y < min_y as isize {
            return false;
        }
        (min_y..=max_y as usize).any(|y| {
            (min_x..=max_x as usize).any(|x| {
                self.alpha
                    .get(y * self.width + x)
                    .is_some_and(|alpha| *alpha > 0)
            })
        })
    }
}

fn topmost_opaque_fish(
    click_pos: Pos2,
    viewport_origin: Pos2,
    fishes: &[FishAnimationState],
    visible_count: usize,
    alpha_masks: &[AlphaMask],
    image_sizes: &[Vec2],
) -> Option<usize> {
    let mut clicked_fish = None;
    for (index, fish) in fishes.iter().take(visible_count).enumerate() {
        let Some(mask) = alpha_masks.get(fish.texture_index) else {
            continue;
        };
        let Some(&image_size) = image_sizes.get(fish.texture_index) else {
            continue;
        };
        let center = viewport_origin + fish.position;
        let size = image_size * fish.scale_pulse;
        let angle = fish.wave_angle.to_radians() * if fish.facing_left { -1.0 } else { 1.0 };
        if mask.contains_opaque_pixel(click_pos, center, size, angle, !fish.facing_left) {
            clicked_fish = Some(index);
        }
    }
    clicked_fish
}

fn spawn_bubbles(
    fish: &mut FishAnimationState,
    center: Pos2,
    size: Vec2,
    now: f64,
) -> Vec<FishBubble> {
    let tail = center.x - fish.horizontal_direction * size.x * 0.34;
    let count = 3 + fish.random_index(5);
    (0..count)
        .map(|_| FishBubble {
            origin: Pos2::new(
                tail + fish.random_between(-3.0, 3.0),
                center.y + fish.random_between(-size.y * 0.06, size.y * 0.06),
            ),
            started_at: now,
            lifetime: fish.random_between(1.1, 1.65),
            horizontal_velocity: -fish.horizontal_direction * fish.random_between(35.0, 65.0),
            rise_speed: fish.random_between(25.0, 42.0),
            radius: fish.random_between(2.5, 5.0),
        })
        .collect()
}

pub struct FishAnimation {
    textures: Vec<TextureHandle>,
    alpha_masks: Vec<AlphaMask>,
    fishes: Vec<FishAnimationState>,
    bubbles: Vec<FishBubble>,
    visible_count: usize,
    last_time: Option<f64>,
    last_viewport: Option<egui::Rect>,
    idle_since: Option<f64>,
    last_pointer: Option<Pos2>,
    orbit_indices: Vec<usize>,
    random: u64,
}

impl FishAnimation {
    pub fn new(ctx: &egui::Context, asset_directories: &[PathBuf]) -> Self {
        let mut textures = Vec::new();
        let mut alpha_masks = Vec::new();
        for (index, name) in ["nemo.png", "fish1.png", "fish2.png", "fish3.png"]
            .into_iter()
            .enumerate()
        {
            let image = asset_directories.iter().find_map(|directory| {
                image::open(directory.join("fish").join(name))
                    .ok()
                    .or_else(|| {
                        (index == 0)
                            .then(|| image::open(directory.join(name)).ok())
                            .flatten()
                    })
            });
            let Some(image) = image else {
                continue;
            };
            let image = image.thumbnail(256, 256).to_rgba8();
            alpha_masks.push(AlphaMask {
                width: image.width() as usize,
                height: image.height() as usize,
                alpha: image.pixels().map(|pixel| pixel[3]).collect(),
            });
            textures.push(ctx.load_texture(
                format!("swimming-fish-{index}"),
                egui::ColorImage::from_rgba_unmultiplied(
                    [image.width() as usize, image.height() as usize],
                    image.as_raw(),
                ),
                egui::TextureOptions::LINEAR,
            ));
        }
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |time| time.as_nanos() as u64);

        let fishes = (0..FISH_COUNT)
            .map(|index| {
                let mut fish = FishAnimationState::new(
                    seed.wrapping_add((index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)),
                );
                if !textures.is_empty() {
                    fish.texture_index = fish.random_index(textures.len());
                }
                fish
            })
            .collect();

        Self {
            textures,
            alpha_masks,
            fishes,
            bubbles: Vec::new(),
            visible_count: FISH_COUNT,
            last_time: None,
            last_viewport: None,
            idle_since: None,
            last_pointer: None,
            orbit_indices: Vec::new(),
            random: seed.max(1),
        }
    }

    fn random_unit(&mut self) -> f32 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        (self.random >> 40) as f32 / 16_777_215.0
    }

    fn start_orbit(&mut self) {
        let count = if self.random_unit() < 0.5 { 2 } else { 3 };
        let mut indices: Vec<_> = (0..self.visible_count).collect();
        for index in (1..indices.len()).rev() {
            let swap_index = (self.random_unit() * (index + 1) as f32) as usize;
            indices.swap(index, swap_index.min(index));
        }
        self.orbit_indices = indices.into_iter().take(count).collect();
        for index in self.orbit_indices.clone() {
            let fish = &mut self.fishes[index];
            fish.orbit_phase = fish.random_between(0.0, std::f32::consts::TAU);
            fish.orbit_radius = fish.random_between(38.0, 76.0);
            let direction = if fish.random_unit() < 0.5 { -1.0 } else { 1.0 };
            fish.orbit_speed = direction * fish.random_between(0.9, 1.4);
        }
    }

    fn update_mouse_idle(&mut self, pointer: Option<Pos2>, viewport: egui::Rect, now: f64) {
        let Some(pointer) = pointer.filter(|position| viewport.contains(*position)) else {
            self.idle_since = None;
            self.last_pointer = None;
            self.orbit_indices.clear();
            return;
        };

        let moved = self
            .last_pointer
            .is_none_or(|last| last.distance_sq(pointer) > 1.0);
        self.last_pointer = Some(pointer);
        if moved {
            self.idle_since = Some(now);
            self.orbit_indices.clear();
            return;
        }

        let idle_since = *self.idle_since.get_or_insert(now);
        if now - idle_since >= MOUSE_IDLE_SECONDS && pointer.y >= viewport.center().y {
            if self.orbit_indices.is_empty() {
                self.start_orbit();
            }
        } else {
            self.orbit_indices.clear();
        }
    }

    pub fn set_density(&mut self, density: EffectDensity) {
        let count = match density {
            EffectDensity::Many => FISH_COUNT,
            EffectDensity::Medium => FISH_COUNT.div_ceil(2),
            EffectDensity::Few => 2,
        }
        .min(self.fishes.len());
        if self.visible_count != count {
            self.visible_count = count;
            self.pause();
        }
    }

    pub fn pause(&mut self) {
        self.last_time = None;
        self.last_viewport = None;
        self.idle_since = None;
        self.last_pointer = None;
        self.orbit_indices.clear();
    }

    pub fn paint(&mut self, ui: &egui::Ui) {
        if self.textures.is_empty() {
            return;
        }
        let viewport = ui.max_rect();
        if viewport.width() < 40.0 || viewport.height() < 40.0 {
            return;
        }

        let now = ui.input(|input| input.time);
        let elapsed = self
            .last_time
            .map_or(0.0, |last| (now - last).max(0.0) as f32);
        self.last_time = Some(now);
        let pointer = ui.input(|input| input.pointer.latest_pos());
        let presses: Vec<_> = ui.input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        ..
                    } => Some(*pos),
                    _ => None,
                })
                .collect()
        });
        self.update_mouse_idle(pointer, viewport, now);

        // Hit-test the frame the user saw, before moving, turning or scaling any fish.
        if let Some(painted_viewport) = self.last_viewport.filter(|_| !presses.is_empty()) {
            let image_sizes: Vec<_> = self.textures.iter().map(TextureHandle::size_vec2).collect();
            for position in presses {
                if !painted_viewport
                    .intersect(ui.clip_rect())
                    .contains(position)
                {
                    continue;
                }
                if let Some(index) = topmost_opaque_fish(
                    position,
                    painted_viewport.left_top(),
                    &self.fishes,
                    self.visible_count,
                    &self.alpha_masks,
                    &image_sizes,
                ) {
                    let fish = &mut self.fishes[index];
                    fish.fast_remaining = CLICK_BOOST_SECONDS;
                    let center = painted_viewport.left_top() + fish.position;
                    let size = image_sizes[fish.texture_index] * fish.scale_pulse;
                    self.bubbles.extend(spawn_bubbles(fish, center, size, now));
                    self.idle_since = Some(now);
                    self.orbit_indices.clear();
                }
            }
        }

        let bounds = viewport.size();
        let safe_margin = (self.textures[0].size_vec2().max_elem() * BASE_SCALE * 0.5 + 76.0)
            .min(bounds.x.min(bounds.y) * 0.35);
        let local_pointer = pointer.map(|position| position - viewport.left_top());
        let orbit_center = local_pointer.map(|position| {
            Vec2::new(
                position
                    .x
                    .clamp(safe_margin, (bounds.x - safe_margin).max(safe_margin)),
                position.y.clamp(
                    viewport.height() * 0.52,
                    (bounds.y - safe_margin).max(viewport.height() * 0.52),
                ),
            )
        });

        for (index, fish) in self.fishes.iter_mut().take(self.visible_count).enumerate() {
            let texture = &self.textures[fish.texture_index];
            let image_size = texture.size_vec2();
            if self.orbit_indices.contains(&index) {
                if let Some(center) = orbit_center {
                    fish.advance_orbit(elapsed, center, bounds, image_size);
                }
            } else {
                fish.advance(elapsed, bounds, image_size);
            }
        }

        for fish in self.fishes.iter().take(self.visible_count) {
            let texture = &self.textures[fish.texture_index];
            let image_size = texture.size_vec2();
            let center = viewport.left_top() + fish.position;
            let size = image_size * fish.scale_pulse;
            let angle = fish.wave_angle.to_radians() * if fish.facing_left { -1.0 } else { 1.0 };
            let (sin, cos) = angle.sin_cos();
            let half_size = size * 0.5;
            let corners = [
                (Vec2::new(-half_size.x, -half_size.y), [0.0, 0.0]),
                (Vec2::new(half_size.x, -half_size.y), [1.0, 0.0]),
                (Vec2::new(half_size.x, half_size.y), [1.0, 1.0]),
                (Vec2::new(-half_size.x, half_size.y), [0.0, 1.0]),
            ];
            let flip = !fish.facing_left;
            let mut mesh = egui::epaint::Mesh::with_texture(texture.id());
            for (corner, uv) in corners {
                let rotated = Vec2::new(
                    corner.x * cos - corner.y * sin,
                    corner.x * sin + corner.y * cos,
                );
                let uv = if flip { [1.0 - uv[0], uv[1]] } else { uv };
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: center + rotated,
                    uv: egui::pos2(uv[0], uv[1]),
                    color: Color32::WHITE,
                });
            }
            mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
            ui.painter().add(egui::Shape::mesh(mesh));
        }
        self.last_viewport = Some(viewport);
        self.bubbles
            .retain(|bubble| now - bubble.started_at < bubble.lifetime as f64);
        for bubble in &self.bubbles {
            bubble.paint(ui.painter(), now);
        }
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn animation_for_tests() -> FishAnimation {
        FishAnimation {
            textures: Vec::new(),
            alpha_masks: Vec::new(),
            fishes: (0..FISH_COUNT)
                .map(|index| FishAnimationState::new(index as u64 + 1))
                .collect(),
            bubbles: Vec::new(),
            visible_count: FISH_COUNT,
            last_time: None,
            last_viewport: None,
            idle_since: None,
            last_pointer: None,
            orbit_indices: Vec::new(),
            random: 17,
        }
    }

    #[test]
    fn pausing_resets_animation_clock_and_mouse_orbit_for_clean_resume() {
        let mut animation = animation_for_tests();
        animation.last_time = Some(100.0);
        animation.idle_since = Some(80.0);
        animation.last_pointer = Some(egui::pos2(600.0, 550.0));
        animation.orbit_indices = vec![0, 1];
        let previous_position = animation.fishes[0].position;
        animation.pause();
        assert!(animation.last_time.is_none());
        assert!(animation.idle_since.is_none());
        assert!(animation.last_pointer.is_none());
        assert!(animation.orbit_indices.is_empty());
        assert_eq!(animation.fishes[0].position, previous_position);
    }

    #[test]
    fn density_limits_fish_counts_and_mouse_orbit_to_visible_fish() {
        let mut animation = animation_for_tests();
        assert_eq!(animation.visible_count, 5);
        animation.set_density(EffectDensity::Medium);
        assert_eq!(animation.visible_count, 3);
        animation.start_orbit();
        assert!(animation.orbit_indices.iter().all(|&index| index < 3));
        animation.set_density(EffectDensity::Few);
        assert_eq!(animation.visible_count, 2);
        assert!(animation.orbit_indices.is_empty());
        animation.start_orbit();
        assert_eq!(animation.orbit_indices.len(), 2);
        assert!(animation.orbit_indices.iter().all(|&index| index < 2));
        animation.set_density(EffectDensity::Many);
        assert_eq!(animation.visible_count, 5);
        assert_eq!(animation.fishes.len(), 5);
    }

    #[test]
    fn fish_moves_and_changes_vertical_direction() {
        let mut fish = FishAnimationState::new(42);
        let bounds = Vec2::new(1200.0, 700.0);
        let size = Vec2::new(100.0, 100.0);
        fish.advance(0.0, bounds, size);
        let initial = fish.position;
        for _ in 0..3600 {
            fish.advance(1.0 / 60.0, bounds, size);
        }
        assert_ne!(fish.position, initial);
        assert!(fish.position.y >= 0.55 * bounds.y);
        assert!((0.35..=0.45).contains(&fish.scale_pulse));
        assert!(fish.wave_angle.abs() <= 8.0);
    }

    #[test]
    fn clicked_fish_swims_faster_for_one_and_a_half_seconds() {
        let bounds = Vec2::new(1200.0, 700.0);
        let size = Vec2::splat(100.0);
        let mut normal = FishAnimationState::new(42);
        let mut boosted = FishAnimationState::new(42);
        for fish in [&mut normal, &mut boosted] {
            fish.initialized = true;
            fish.position = Vec2::new(500.0, 500.0);
            fish.horizontal_direction = 1.0;
            fish.horizontal_speed = 2.0;
        }
        boosted.fast_remaining = CLICK_BOOST_SECONDS;
        normal.advance(0.05, bounds, size);
        boosted.advance(0.05, bounds, size);
        assert!(boosted.position.x - 500.0 > (normal.position.x - 500.0) * 2.7);
        assert!((boosted.fast_remaining - 1.45).abs() < 0.001);
    }

    #[test]
    fn emitted_bubbles_travel_behind_then_rise_and_fade() {
        let mut fish = FishAnimationState::new(42);
        fish.horizontal_direction = 1.0;
        let center = Pos2::new(400.0, 500.0);
        let bubbles = spawn_bubbles(&mut fish, center, Vec2::new(100.0, 60.0), 10.0);
        assert!((3..=7).contains(&bubbles.len()));
        assert!(bubbles
            .iter()
            .all(|bubble| bubble.horizontal_velocity < 0.0));
        for bubble in &bubbles {
            let midpoint = 10.0 + (bubble.lifetime * 0.5) as f64;
            let after_kick = bubble.position(midpoint);
            assert!(after_kick.x < bubble.origin.x);
            assert_eq!(after_kick.y, bubble.origin.y);
            let after_rise = bubble.position(midpoint + 0.1);
            assert_eq!(after_rise.x, after_kick.x);
            assert!(after_rise.y < bubble.origin.y);
            assert_eq!(bubble.opacity(10.0 + bubble.lifetime as f64), 0.0);
        }
    }

    #[test]
    fn transparent_pixels_do_not_hit_and_overlaps_choose_topmost_visible_fish() {
        let opaque_mask = || AlphaMask {
            width: 10,
            height: 10,
            alpha: vec![255; 100],
        };
        let mut fish = vec![FishAnimationState::new(1), FishAnimationState::new(2)];
        for (index, state) in fish.iter_mut().enumerate() {
            state.texture_index = index;
            state.position = Vec2::new(100.0, 100.0);
            state.scale_pulse = 1.0;
            state.initialized = true;
        }
        let point = Pos2::new(100.0, 100.0);
        let masks = vec![opaque_mask(), opaque_mask()];
        let image_sizes = vec![Vec2::splat(10.0); 2];
        assert_eq!(
            topmost_opaque_fish(point, Pos2::ZERO, &fish, 2, &masks, &image_sizes),
            Some(1)
        );
        assert_eq!(
            topmost_opaque_fish(point, Pos2::ZERO, &fish, 1, &masks, &image_sizes),
            Some(0)
        );

        let transparent_top = AlphaMask {
            width: 10,
            height: 10,
            alpha: vec![0; 100],
        };
        assert!(!transparent_top.contains_opaque_pixel(
            point,
            point,
            Vec2::splat(10.0),
            0.0,
            false,
        ));
        assert_eq!(
            topmost_opaque_fish(
                point,
                Pos2::ZERO,
                &fish,
                2,
                &[opaque_mask(), transparent_top],
                &image_sizes,
            ),
            Some(0)
        );
    }

    #[test]
    fn alpha_hit_test_tracks_source_pixels_after_flip_and_rotation() {
        let mut alpha = vec![0; 8];
        alpha[1] = 255;
        let mask = AlphaMask {
            width: 4,
            height: 2,
            alpha,
        };
        let center = Pos2::new(100.0, 100.0);
        let size = Vec2::new(40.0, 20.0);
        assert!(mask.contains_opaque_pixel(Pos2::new(95.0, 95.0), center, size, 0.0, false,));
        assert!(!mask.contains_opaque_pixel(Pos2::new(115.0, 95.0), center, size, 0.0, false,));
        assert!(mask.contains_opaque_pixel(Pos2::new(105.0, 95.0), center, size, 0.0, true,));
        assert!(mask.contains_opaque_pixel(
            Pos2::new(105.0, 95.0),
            center,
            size,
            std::f32::consts::FRAC_PI_2,
            false,
        ));
    }

    #[test]
    fn thin_translucent_edges_are_clickable_without_hitting_empty_padding() {
        let mut alpha = vec![0; 100 * 50];
        alpha[25 * 100 + 50] = 20;
        let mask = AlphaMask {
            width: 100,
            height: 50,
            alpha,
        };
        let center = Pos2::new(100.0, 100.0);
        for flip in [false, true] {
            for angle in [0.0, 0.14, -0.14, std::f32::consts::FRAC_PI_2] {
                let local = Vec2::new(if flip { -2.0 } else { 2.0 }, 0.5);
                let (sin, cos) = angle.sin_cos();
                let point = center
                    + Vec2::new(local.x * cos - local.y * sin, local.x * sin + local.y * cos);
                assert!(mask.contains_opaque_pixel(
                    point,
                    center,
                    Vec2::new(100.0, 50.0),
                    angle,
                    flip
                ));
                assert!(!mask.contains_opaque_pixel(
                    center + Vec2::new(20.0, 0.0),
                    center,
                    Vec2::new(100.0, 50.0),
                    angle,
                    flip
                ));
            }
        }
    }

    #[test]
    fn pressing_the_displayed_tail_boosts_immediately_even_with_overlapping_fish() {
        for count in [1, 2] {
            let ctx = egui::Context::default();
            let mut animation = animation_for_tests();
            animation.visible_count = count;
            animation.textures.push(ctx.load_texture(
                "click-test-fish",
                egui::ColorImage::new([256, 128], vec![Color32::WHITE; 256 * 128]),
                egui::TextureOptions::LINEAR,
            ));
            animation.alpha_masks.push(AlphaMask {
                width: 256,
                height: 128,
                alpha: vec![255; 256 * 128],
            });
            for fish in &mut animation.fishes {
                fish.initialized = true;
                fish.position = Vec2::new(500.0, 600.0);
                fish.route_target_y = 600.0;
                fish.horizontal_speed = 3.0;
            }
            let screen = egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0));
            let _ = crate::test_support::run_ui(
                &ctx,
                egui::RawInput {
                    time: Some(0.0),
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ui| animation.paint(ui),
            );
            let viewport = animation.last_viewport.unwrap();
            let fish = &animation.fishes[count - 1];
            let tail =
                viewport.min + fish.position - Vec2::new(256.0 * fish.scale_pulse * 0.5 - 0.5, 0.0);
            if count == 2 {
                animation.last_pointer = Some(tail);
                animation.idle_since = Some(-60.0);
                animation.orbit_indices = vec![0, 1];
                for fish in &mut animation.fishes[..count] {
                    fish.orbiting = true;
                }
            }
            // A quick press/release must still use the displayed frame, before this frame's movement.
            let button = |pressed| egui::Event::PointerButton {
                pos: tail,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            let _ = crate::test_support::run_ui(
                &ctx,
                egui::RawInput {
                    time: Some(0.05),
                    screen_rect: Some(screen),
                    events: vec![egui::Event::PointerMoved(tail), button(true), button(false)],
                    ..Default::default()
                },
                |ui| animation.paint(ui),
            );
            assert!(animation.fishes[count - 1].fast_remaining > 1.4);
            assert!(animation.fishes[count - 1].position.x > 500.0 + 3.0 * 3.0 * 2.7);
            assert!((3..=7).contains(&animation.bubbles.len()));
            assert!(animation.orbit_indices.is_empty());
            assert!(!animation.fishes[count - 1].orbiting);
            if count == 2 {
                assert_eq!(animation.fishes[0].fast_remaining, 0.0);
            }

            // A held press also triggers immediately, without waiting for release.
            let fish = &animation.fishes[count - 1];
            let center = viewport.min + fish.position;
            let _ = crate::test_support::run_ui(
                &ctx,
                egui::RawInput {
                    time: Some(0.10),
                    screen_rect: Some(screen),
                    events: vec![egui::Event::PointerButton {
                        pos: center,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    }],
                    ..Default::default()
                },
                |ui| animation.paint(ui),
            );
            assert!((animation.fishes[count - 1].fast_remaining - 1.45).abs() < 0.001);
            let _ = crate::test_support::run_ui(
                &ctx,
                egui::RawInput {
                    time: Some(0.15),
                    screen_rect: Some(screen),
                    events: vec![egui::Event::PointerButton {
                        pos: center,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    }],
                    ..Default::default()
                },
                |ui| animation.paint(ui),
            );
            assert!((animation.fishes[count - 1].fast_remaining - 1.40).abs() < 0.001);
        }
    }

    #[test]
    fn fish_bounces_at_horizontal_margins() {
        let mut fish = FishAnimationState::new(7);
        fish.initialized = true;
        fish.position = Vec2::new(1099.0, 350.0);
        fish.horizontal_direction = 1.0;
        let previous_lane = fish.route_lane;
        fish.advance(1.0 / 60.0, Vec2::new(1200.0, 700.0), Vec2::splat(100.0));
        assert_eq!(fish.horizontal_direction, -1.0);
        assert_ne!(fish.route_lane, previous_lane);
        assert!(fish.facing_left);
    }

    #[test]
    fn fish_start_at_random_positions_in_the_lower_half() {
        let bounds = Vec2::new(1200.0, 700.0);
        let size = Vec2::splat(100.0);
        let mut fishes: Vec<_> = (0..FISH_COUNT)
            .map(|index| FishAnimationState::new(index as u64 + 1))
            .collect();

        for fish in &mut fishes {
            fish.advance(0.0, bounds, size);
            assert!(fish.position.y >= bounds.y * 0.55);
        }
        assert!(fishes
            .windows(2)
            .any(|pair| pair[0].position != pair[1].position));
    }

    #[test]
    fn idle_mouse_in_lower_half_starts_two_or_three_orbiting_fish() {
        let mut animation = animation_for_tests();
        let viewport = egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 800.0));
        let pointer = egui::pos2(450.0, 650.0);

        animation.update_mouse_idle(Some(pointer), viewport, 0.0);
        animation.update_mouse_idle(Some(pointer), viewport, 59.9);
        assert!(animation.orbit_indices.is_empty());

        animation.update_mouse_idle(Some(pointer), viewport, 60.0);
        assert!((2..=3).contains(&animation.orbit_indices.len()));
    }

    #[test]
    fn mouse_movement_resets_idle_orbit() {
        let mut animation = animation_for_tests();
        let viewport = egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 800.0));
        let pointer = egui::pos2(450.0, 650.0);

        animation.update_mouse_idle(Some(pointer), viewport, 0.0);
        animation.update_mouse_idle(Some(pointer), viewport, 60.0);
        assert!(!animation.orbit_indices.is_empty());

        let moved_pointer = pointer + Vec2::new(2.0, 0.0);
        animation.update_mouse_idle(Some(moved_pointer), viewport, 61.0);
        assert!(animation.orbit_indices.is_empty());
        animation.update_mouse_idle(Some(moved_pointer), viewport, 120.9);
        assert!(animation.orbit_indices.is_empty());
        animation.update_mouse_idle(Some(moved_pointer), viewport, 121.0);
        assert!((2..=3).contains(&animation.orbit_indices.len()));
    }

    #[test]
    fn idle_mouse_in_upper_half_does_not_start_orbit() {
        let mut animation = animation_for_tests();
        let viewport = egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 800.0));
        let pointer = egui::pos2(450.0, 200.0);

        animation.update_mouse_idle(Some(pointer), viewport, 0.0);
        animation.update_mouse_idle(Some(pointer), viewport, 120.0);
        assert!(animation.orbit_indices.is_empty());
    }
}
