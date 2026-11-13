//! A uniform square lattice projected into the existing disco-floor perspective.
use crate::audio_analysis::AudioReactive;
use eframe::egui::{self, Color32, Pos2, Rect, Shape};

const COLUMNS: usize = 26;
const ROWS: usize = 16;
const HORIZON: f32 = 0.57;

#[derive(Clone)]
struct Tile {
    x: usize,
    y: usize,
    hue: f32,
    previous: Color32,
    color: Color32,
    changed: f64,
    next_change: f64,
    transition: f32,
    flash: f32,
}

pub struct DiscoFloor {
    tiles: Vec<Tile>,
    random: u64,
    last_beat: u64,
    was_lit: bool,
}

impl DiscoFloor {
    pub fn new() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |time| time.as_nanos() as u64)
            .max(1);
        let mut floor = Self {
            tiles: Vec::with_capacity(COLUMNS * ROWS),
            random: seed,
            last_beat: 0,
            was_lit: false,
        };
        for y in 0..ROWS {
            for x in 0..COLUMNS {
                let hue = floor.random_unit();
                let next_change = f64::from(floor.random_unit());
                floor.tiles.push(Tile {
                    x,
                    y,
                    hue,
                    previous: Color32::BLACK,
                    color: Color32::BLACK,
                    changed: 0.0,
                    next_change,
                    transition: 0.3,
                    flash: 0.0,
                });
            }
        }
        floor
    }

    pub fn pause(&mut self) {
        self.was_lit = false;
    }

    fn random_unit(&mut self) -> f32 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        (self.random >> 40) as f32 / 16_777_216.0
    }

    fn update(&mut self, now: f64, dt: f32, audio: AudioReactive) -> bool {
        let lit = audio.energy > 0.008;
        let energy = audio.energy.clamp(0.0, 1.0);
        let beat = audio.beat_count != self.last_beat;
        self.last_beat = audio.beat_count;
        let interval =
            ((60.0 / audio.tempo_bpm.clamp(45.0, 200.0)) * (1.4 - energy * 1.1)).clamp(0.10, 1.8);
        let waking = lit && !self.was_lit;
        for index in 0..self.tiles.len() {
            let beat_chance = if beat && lit { self.random_unit() } else { 1.0 };
            if waking {
                self.tiles[index].next_change = now + f64::from(self.random_unit()) * 0.8;
            } else if beat_chance < 0.18 + energy * 0.60 {
                let delay = f64::from(self.random_unit()) * f64::from(0.07 + (1.0 - energy) * 0.28);
                self.tiles[index].next_change = self.tiles[index].next_change.min(now + delay);
            }
            if lit && now >= self.tiles[index].next_change {
                let new_hue = self.random_unit();
                let saturation = 0.64 + self.random_unit() * 0.30;
                let value = 0.48 + energy * 0.31 + self.random_unit() * 0.16;
                let next = now + f64::from(interval * (0.65 + self.random_unit() * 1.65));
                let tile = &mut self.tiles[index];
                tile.hue = new_hue;
                tile.previous = tile_color(tile, now);
                tile.color =
                    egui::ecolor::Hsva::new(new_hue, saturation, value.min(1.0), 1.0).into();
                tile.changed = now;
                tile.transition = 0.42 - energy * 0.34;
                tile.next_change = next;
                tile.flash = audio.beat.max(0.3);
            }
            self.tiles[index].flash =
                (self.tiles[index].flash - dt.max(0.0) * (1.5 + energy * 3.0)).max(0.0);
        }
        self.was_lit = lit;
        lit
    }

    pub fn paint(
        &mut self,
        painter: &egui::Painter,
        bounds: Rect,
        now: f64,
        dt: f32,
        audio: AudioReactive,
    ) {
        if !bounds.is_positive() {
            return;
        }
        let lit = self.update(now, dt, audio);
        let painter = painter.with_clip_rect(painter.clip_rect().intersect(bounds));
        let mut shapes = Vec::with_capacity(self.tiles.len() + 1);
        shapes.push(Shape::convex_polygon(
            vec![
                project(bounds, 0.0, 0.0),
                project(bounds, COLUMNS as f32, 0.0),
                project(bounds, COLUMNS as f32, ROWS as f32),
                project(bounds, 0.0, ROWS as f32),
            ],
            Color32::from_rgba_unmultiplied(7, 8, 14, 215),
            egui::Stroke::NONE,
        ));
        for tile in &self.tiles {
            // Every tile occupies exactly one world-grid cell. Apparent size
            // changes only with perspective, never by combining adjacent cells.
            let inset = 0.045;
            let x = tile.x as f32;
            let y = tile.y as f32;
            let base = if lit {
                tile_color(tile, now)
            } else {
                Color32::from_rgb(38, 40, 48)
            };
            let shade = (0.42 + (y / ROWS as f32) * 0.45) * (0.94 + tile.hue * 0.06);
            let brightness = if lit { shade + tile.flash * 0.13 } else { 0.78 };
            let color = Color32::from_rgba_unmultiplied(
                (f32::from(base.r()) * brightness).min(255.0) as u8,
                (f32::from(base.g()) * brightness).min(255.0) as u8,
                (f32::from(base.b()) * brightness).min(255.0) as u8,
                if lit { 192 } else { 210 },
            );
            shapes.push(Shape::convex_polygon(
                vec![
                    project(bounds, x + inset, y + inset),
                    project(bounds, x + 1.0 - inset, y + inset),
                    project(bounds, x + 1.0 - inset, y + 1.0 - inset),
                    project(bounds, x + inset, y + 1.0 - inset),
                ],
                color,
                egui::Stroke::new(0.45, Color32::from_white_alpha(if lit { 18 } else { 10 })),
            ));
        }
        painter.add(Shape::Vec(shapes));
    }
}

fn project(bounds: Rect, x: f32, y: f32) -> Pos2 {
    let depth = (y / ROWS as f32).powf(1.65);
    let top = bounds.top() + bounds.height() * HORIZON;
    Pos2::new(
        bounds.center().x + (x / COLUMNS as f32 - 0.5) * bounds.width() * (1.08 + depth * 0.72),
        top + (bounds.bottom() - top) * depth,
    )
}

fn tile_color(tile: &Tile, now: f64) -> Color32 {
    let t = ((now - tile.changed) as f32 / tile.transition.max(0.01)).clamp(0.0, 1.0);
    let t = t * t * (3.0 - 2.0 * t);
    let previous = tile.previous.to_array();
    let color = tile.color.to_array();
    Color32::from_rgb(
        egui::lerp(f32::from(previous[0])..=f32::from(color[0]), t) as u8,
        egui::lerp(f32::from(previous[1])..=f32::from(color[1]), t) as u8,
        egui::lerp(f32::from(previous[2])..=f32::from(color[2]), t) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Vec2;

    #[test]
    fn uniform_grid_has_every_single_cell_once_without_merged_squares() {
        let floor = DiscoFloor::new();
        assert_eq!(floor.tiles.len(), COLUMNS * ROWS);
        let mut occupied = [[false; COLUMNS]; ROWS];
        for tile in &floor.tiles {
            assert!(!occupied[tile.y][tile.x]);
            occupied[tile.y][tile.x] = true;
        }
        assert!(occupied.iter().flatten().all(|value| *value));
        let bounds = Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 790.0));
        for row in 0..ROWS {
            let width = project(bounds, 1.0, row as f32).x - project(bounds, 0.0, row as f32).x;
            for column in 1..COLUMNS {
                let next_width = project(bounds, column as f32 + 1.0, row as f32).x
                    - project(bounds, column as f32, row as f32).x;
                assert!((width - next_width).abs() < 0.001);
            }
        }
    }

    #[test]
    fn far_edge_and_all_depths_cover_full_viewport_while_retaining_perspective() {
        for size in [
            Vec2::new(950.0, 610.0),
            Vec2::new(1280.0, 790.0),
            Vec2::new(3840.0, 2160.0),
        ] {
            let bounds = Rect::from_min_size(Pos2::new(7.0, 11.0), size);
            for row in 0..=ROWS {
                let left = project(bounds, 0.0, row as f32);
                let right = project(bounds, COLUMNS as f32, row as f32);
                assert!(left.x < bounds.left());
                assert!(right.x > bounds.right());
            }
            assert!(
                (project(bounds, 0.0, 0.0).y - (bounds.top() + size.y * HORIZON)).abs() < 0.001
            );
            let far_width = project(bounds, COLUMNS as f32, 0.0).x - project(bounds, 0.0, 0.0).x;
            let near_width = project(bounds, COLUMNS as f32, ROWS as f32).x
                - project(bounds, 0.0, ROWS as f32).x;
            assert!(near_width > far_width * 1.5);
            assert_eq!(project(bounds, 0.0, ROWS as f32).y, bounds.bottom());
        }
    }

    #[test]
    fn beat_changes_remain_asynchronous_and_fast_music_changes_more_often() {
        let simulate = |energy: f32, tempo: f32| {
            let mut floor = DiscoFloor::new();
            floor.random = 42;
            let mut changes = 0;
            let mut previous = vec![0.0; floor.tiles.len()];
            for frame in 0..600 {
                let now = frame as f64 / 60.0;
                floor.update(
                    now,
                    1.0 / 60.0,
                    AudioReactive {
                        energy,
                        beat: 0.8,
                        beat_count: (now * f64::from(tempo) / 60.0) as u64,
                        tempo_bpm: tempo,
                    },
                );
                for (tile, last) in floor.tiles.iter().zip(&mut previous) {
                    if tile.changed != *last {
                        changes += 1;
                        *last = tile.changed;
                    }
                }
            }
            assert!(floor
                .tiles
                .windows(2)
                .any(|pair| (pair[0].next_change - pair[1].next_change).abs() > 0.02));
            assert!(floor.tiles.iter().all(|tile| tile.color != Color32::BLACK));
            changes
        };
        assert!(simulate(0.9, 150.0) > simulate(0.08, 55.0) * 2);
    }

    #[test]
    fn pause_immediately_draws_only_gray_unlit_cells() {
        let ctx = egui::Context::default();
        let mut floor = DiscoFloor::new();
        floor.random = 42;
        floor.update(
            1.0,
            0.016,
            AudioReactive {
                energy: 0.9,
                beat: 1.0,
                beat_count: 1,
                tempo_bpm: 150.0,
            },
        );
        floor.update(
            2.0,
            0.016,
            AudioReactive {
                energy: 0.9,
                beat: 1.0,
                beat_count: 2,
                tempo_bpm: 150.0,
            },
        );
        let output = crate::test_support::run_ui(&ctx, egui::RawInput::default(), |ui| {
            floor.paint(
                ui.painter(),
                Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 790.0)),
                2.1,
                0.016,
                AudioReactive::default(),
            );
        });
        let mut colors = Vec::new();
        fn collect(shape: &Shape, colors: &mut Vec<Color32>) {
            match shape {
                Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, colors);
                    }
                }
                Shape::Path(path) if path.fill.a() == 210 => colors.push(path.fill),
                _ => {}
            }
        }
        for shape in output.shapes {
            collect(&shape.shape, &mut colors);
        }
        assert_eq!(colors.len(), COLUMNS * ROWS);
        assert!(colors.iter().all(|color| *color == colors[0]));
        assert!(colors[0].r() <= colors[0].g() && colors[0].g() <= colors[0].b());
        assert!(!floor.was_lit);
        floor.pause();
        assert!(!floor.was_lit);
    }
}
