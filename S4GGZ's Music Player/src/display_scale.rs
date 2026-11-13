//! Keep the same design proportions on different monitor resolutions and DPI.
//!
//! The root window owns egui's global zoom, so its child equalizer shares it.
use eframe::egui::{self, Vec2};

const REFERENCE_MONITOR: Vec2 = Vec2::new(1920.0, 1080.0);
const INITIAL_SIZE: Vec2 = Vec2::new(1280.0, 790.0);
const MINIMUM_SIZE: Vec2 = Vec2::new(950.0, 610.0);
const MIN_PIXELS_PER_POINT: f32 = 0.5;

#[derive(Clone, Copy, Debug)]
struct Monitor {
    physical_size: Vec2,
    native_scale: f32,
}

impl Monitor {
    fn usable_pixels(self) -> Vec2 {
        // Native chrome follows Windows DPI, independently of our design scale.
        (self.physical_size - Vec2::new(32.0, 80.0) * self.native_scale).max(Vec2::splat(1.0))
    }

    fn pixels_per_point(self) -> f32 {
        let proportional = (self.physical_size / REFERENCE_MONITOR).min_elem();
        // At 360p/480p with high OS scaling, also fit the whole design below
        // the native title bar and taskbar rather than clipping its controls.
        let fitted = proportional.min((self.usable_pixels() / INITIAL_SIZE).min_elem());
        // Preserve a minimum physical text size on very small displays. Scrollable
        // panels handle the reduced viewport instead of shrinking every label below
        // a readable size.
        fitted.max(MIN_PIXELS_PER_POINT)
    }

    fn zoom(self) -> f32 {
        self.pixels_per_point() / self.native_scale
    }

    fn usable_size(self) -> Vec2 {
        // Leave room for the native title bar and taskbar even on small displays.
        self.usable_pixels() / self.pixels_per_point()
    }

    fn same_as(self, other: Self) -> bool {
        (self.physical_size - other.physical_size).abs().max_elem() < 2.0
            && (self.native_scale - other.native_scale).abs() < 0.001
    }
}

#[derive(Default)]
pub(super) struct DisplayScale {
    monitor: Option<Monitor>,
    normal_size: Option<Vec2>,
    pending_size: Option<Vec2>,
    resize_grace_passes: u8,
}

impl DisplayScale {
    /// Call from App::raw_input_hook, before egui starts the root pass.
    pub(super) fn prepare(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if raw.viewport_id != egui::ViewportId::ROOT {
            return;
        }
        let viewport = raw.viewport();
        let Some(size) = viewport
            .monitor_size
            .filter(|size| size.is_finite() && size.x > 0.0 && size.y > 0.0)
        else {
            return;
        };
        let native_scale = viewport.native_pixels_per_point.unwrap_or(1.0);
        let old_zoom = ctx.zoom_factor();
        if !native_scale.is_finite()
            || native_scale <= 0.0
            || !old_zoom.is_finite()
            || old_zoom <= 0.0
        {
            return;
        }
        // Backend monitor_size is ALREADY divided by both DPI and current zoom.
        // Recover physical pixels first: directly using size/reference oscillates.
        let monitor = Monitor {
            physical_size: (size * (native_scale * old_zoom)).round(),
            native_scale,
        };
        if self
            .monitor
            .is_none_or(|previous| !monitor.same_as(previous))
        {
            self.pending_size = Some(self.normal_size.unwrap_or(INITIAL_SIZE));
            self.monitor = Some(monitor);
        }
        let zoom = monitor.zoom();
        if (zoom - old_zoom).abs() > 0.0001 {
            // Being outside the pass makes this active before App::logic/ui.
            ctx.set_zoom_factor(zoom);
        }
    }

    /// Call from App::logic. Resize only at startup or on a monitor/DPI change.
    pub(super) fn apply(&mut self, ctx: &egui::Context) {
        if ctx.viewport_id() != egui::ViewportId::ROOT {
            return;
        }
        let Some(monitor) = self.monitor else {
            return;
        };
        let normal = ctx.input(|input| {
            input.viewport().maximized != Some(true)
                && input.viewport().fullscreen != Some(true)
                && input.viewport().minimized != Some(true)
        });
        if let Some(size) = self.pending_size.take() {
            let usable = monitor.usable_size();
            let minimum = MINIMUM_SIZE.min(usable);
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(minimum));
            let size = size.max(minimum).min(usable);
            self.normal_size = Some(size);
            if normal {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                // Native resizing is asynchronous. Do not record the old window
                // size as a manual resize while the requested size is arriving.
                self.resize_grace_passes = 2;
            }
        } else if self.resize_grace_passes > 0 {
            self.resize_grace_passes -= 1;
        } else if normal {
            let size = ctx.input(|input| input.content_rect().size());
            if size.is_finite() && size.x > 0.0 && size.y > 0.0 {
                self.normal_size = Some(size);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(
        scaling: &mut DisplayScale,
        ctx: &egui::Context,
        physical_monitor: Option<Vec2>,
        native_scale: f32,
        physical_window: Vec2,
        maximized: bool,
    ) -> egui::FullOutput {
        let ppp = ctx.zoom_factor() * native_scale;
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                physical_window / ppp,
            )),
            ..Default::default()
        };
        let root = input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap();
        root.monitor_size = physical_monitor.map(|size| size / ppp);
        root.native_pixels_per_point = Some(native_scale);
        root.maximized = Some(maximized);
        scaling.prepare(ctx, &mut input);
        crate::test_support::run_ui(ctx, input, |ui| scaling.apply(ui.ctx()))
    }

    fn resized(output: &egui::FullOutput) -> Option<Vec2> {
        output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .find_map(|command| {
                if let egui::ViewportCommand::InnerSize(size) = command {
                    Some(*size)
                } else {
                    None
                }
            })
    }

    #[test]
    fn monitor_proportions_match_from_360p_to_4k_at_every_windows_dpi() {
        for resolution in [
            Vec2::new(640.0, 360.0),
            Vec2::new(640.0, 480.0),
            Vec2::new(854.0, 480.0),
            Vec2::new(1280.0, 720.0),
            Vec2::new(1366.0, 768.0),
            Vec2::new(1920.0, 1080.0),
            Vec2::new(2560.0, 1440.0),
            Vec2::new(3840.0, 2160.0),
        ] {
            for dpi in [1.0, 1.5, 2.0] {
                let ctx = egui::Context::default();
                let mut scale = DisplayScale::default();
                let output = render(&mut scale, &ctx, Some(resolution), dpi, INITIAL_SIZE, false);
                let initial = resized(&output).unwrap();
                let proportional = (resolution / REFERENCE_MONITOR).min_elem();
                let usable = resolution - Vec2::new(32.0, 80.0) * dpi;
                let desired_ppp = proportional
                    .min((usable / INITIAL_SIZE).min_elem())
                    .max(MIN_PIXELS_PER_POINT);
                assert!((ctx.pixels_per_point() - desired_ppp).abs() < 0.001);
                let usable_size = usable / desired_ppp;
                let minimum = MINIMUM_SIZE.min(usable_size);
                let expected_size = INITIAL_SIZE.max(minimum).min(usable_size);
                assert!((initial - expected_size).abs().max_elem() < 0.01);
                let physical_size = initial * ctx.pixels_per_point();
                assert!(physical_size.x <= usable.x + 0.01);
                assert!(physical_size.y <= usable.y + 0.01);
                if desired_ppp == proportional {
                    assert!(
                        (physical_size.x / physical_size.y - INITIAL_SIZE.x / INITIAL_SIZE.y).abs()
                            < 0.001
                    );
                    // A 4:3 monitor is width-limited; it has extra vertical room.
                    let width_limited =
                        resolution.x / resolution.y < REFERENCE_MONITOR.x / REFERENCE_MONITOR.y;
                    let (actual_fraction, reference_fraction) = if width_limited {
                        (
                            physical_size.x / resolution.x,
                            INITIAL_SIZE.x / REFERENCE_MONITOR.x,
                        )
                    } else {
                        (
                            physical_size.y / resolution.y,
                            INITIAL_SIZE.y / REFERENCE_MONITOR.y,
                        )
                    };
                    assert!((actual_fraction - reference_fraction).abs() < 0.001);
                }
                for _ in 0..5 {
                    let output = render(
                        &mut scale,
                        &ctx,
                        Some(resolution),
                        dpi,
                        initial * desired_ppp,
                        false,
                    );
                    assert!(resized(&output).is_none(), "stable monitor must not resize");
                    assert!((ctx.pixels_per_point() - desired_ppp).abs() < 0.001);
                }
            }
        }
    }

    #[test]
    fn moving_to_another_monitor_preserves_manual_window_proportions() {
        let ctx = egui::Context::default();
        let mut scale = DisplayScale::default();
        let hd = Vec2::new(1920.0, 1080.0);
        render(&mut scale, &ctx, Some(hd), 1.0, INITIAL_SIZE, false);
        for _ in 0..3 {
            render(&mut scale, &ctx, Some(hd), 1.0, INITIAL_SIZE, false);
        }
        let custom = Vec2::new(1450.0, 900.0);
        let output = render(&mut scale, &ctx, Some(hd), 1.0, custom, false);
        assert!(resized(&output).is_none());
        let output = render(
            &mut scale,
            &ctx,
            Some(Vec2::new(3840.0, 2160.0)),
            1.5,
            custom,
            false,
        );
        assert_eq!(resized(&output), Some(custom));
        assert!((ctx.pixels_per_point() - 2.0).abs() < 0.001);
    }

    #[test]
    fn maximize_and_missing_monitor_do_not_force_a_window_resize() {
        let ctx = egui::Context::default();
        let mut scale = DisplayScale::default();
        let output = render(&mut scale, &ctx, None, 1.0, INITIAL_SIZE, false);
        assert!(resized(&output).is_none());
        assert_eq!(ctx.zoom_factor(), 1.0);
        let output = render(
            &mut scale,
            &ctx,
            Some(Vec2::new(3840.0, 2160.0)),
            2.0,
            INITIAL_SIZE,
            true,
        );
        assert!(resized(&output).is_none());
        assert!(output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .any(|command| matches!(command, egui::ViewportCommand::MinInnerSize(_))));
    }
}
