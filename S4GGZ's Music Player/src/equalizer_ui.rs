//! The vertical ten-band editor lives in a child viewport owned by the player.
//! Closing the root window also destroys this viewport; no helper process runs.
use crate::{
    equalizer::{Equalizer, FilterType, Preset},
    AuraApp, Duration, Instant, MUTED,
};
use eframe::egui::{self, RichText, Vec2};

pub(super) fn viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("equalizer-window")
}

fn frequency_label(frequency: f32) -> String {
    if frequency >= 1000. {
        format!("{:.1}k", frequency / 1000.)
    } else {
        format!("{frequency:.0}")
    }
}

fn fader(
    ui: &mut egui::Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    name: &str,
) -> egui::Response {
    let minimum = *range.start();
    let maximum = *range.end();
    const HANDLE_ASPECT: f32 = 0.9;
    let response = ui
        .scope(|ui| {
            ui.spacing_mut().slider_width = 180.;
            // Keep egui's pointer/keyboard/accessibility behavior and paint a
            // separate dark rail and silver handle, as on a physical equalizer.
            let widgets = &mut ui.visuals_mut().widgets;
            for visual in [
                &mut widgets.inactive,
                &mut widgets.hovered,
                &mut widgets.active,
            ] {
                visual.bg_fill = egui::Color32::TRANSPARENT;
                visual.fg_stroke = egui::Stroke::NONE;
            }
            ui.add_sized(
                Vec2::new(24., 180.),
                egui::Slider::new(value, range)
                    .vertical()
                    .show_value(false)
                    .trailing_fill(false)
                    .step_by(0.1)
                    .handle_shape(egui::style::HandleShape::Rect {
                        aspect_ratio: HANDLE_ASPECT,
                    }),
            )
        })
        .inner
        .on_hover_text(name);
    let rect = response.rect;
    let painter = ui.painter();
    painter.rect_filled(
        egui::Rect::from_center_size(rect.center(), Vec2::new(3., rect.height())),
        1,
        egui::Color32::from_rgb(5, 10, 18),
    );
    // egui's rectangular handle shrinks its travel by width / 2.5 * aspect.
    // Using that same travel keeps the artwork aligned with the actual hit area.
    let half_height = rect.width() / 2.5 * HANDLE_ASPECT;
    for division in 0..=4 {
        let y = egui::lerp(
            (rect.bottom() - half_height)..=(rect.top() + half_height),
            division as f32 / 4.,
        );
        painter.line_segment(
            [
                egui::pos2(rect.center().x - 6., y),
                egui::pos2(rect.center().x + 6., y),
            ],
            egui::Stroke::new(1., egui::Color32::from_white_alpha(45)),
        );
    }
    let fraction = ((*value - minimum) / (maximum - minimum)).clamp(0., 1.);
    let y = egui::lerp(
        (rect.bottom() - half_height)..=(rect.top() + half_height),
        fraction,
    );
    let handle = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, y),
        Vec2::new(rect.width(), 2. * half_height),
    );
    painter.rect_filled(
        handle.translate(Vec2::new(0., 1.)),
        2,
        egui::Color32::from_black_alpha(100),
    );
    painter.rect_filled(handle, 2, egui::Color32::from_rgb(198, 209, 220));
    painter.line_segment(
        [
            handle.left_top() + Vec2::new(2., 1.),
            handle.right_top() + Vec2::new(-2., 1.),
        ],
        egui::Stroke::new(1., crate::WHITE),
    );
    let edge = if response.hovered() || response.has_focus() {
        crate::theme::accent(ui.ctx())
    } else {
        egui::Color32::from_rgb(100, 115, 130)
    };
    painter.rect_stroke(
        handle,
        2,
        egui::Stroke::new(1., edge),
        egui::StrokeKind::Inside,
    );
    painter.line_segment(
        [
            egui::pos2(handle.left() + 3., y),
            egui::pos2(handle.right() - 3., y),
        ],
        egui::Stroke::new(1., egui::Color32::from_rgb(70, 85, 100)),
    );
    response.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(*value), name));
    response
}

impl AuraApp {
    pub(super) fn equalizer_window(&mut self, ctx: &egui::Context) {
        if !self.equalizer_open {
            self.equalizer_fish.pause();
            return;
        }
        ctx.show_viewport_immediate(
            viewport_id(),
            egui::ViewportBuilder::default()
                .with_title("S4GGZ — Egalizator")
                .with_inner_size([820., 500.])
                .with_min_inner_size([760., 500.]),
            |ui, _class| {
                if ui.input(|input| input.viewport().close_requested()) {
                    self.equalizer_open = false;
                    self.equalizer_fish.pause();
                    return;
                }
                self.equalizer_surface(ui);
            },
        );
    }

    pub(super) fn equalizer_surface(&mut self, ui: &mut egui::Ui) {
        // Share the active theme's already-loaded image with the main player.
        // Paint it on the viewport's background before any panel or control;
        // popups retain egui's foreground layers above this image.
        crate::paint_ambient(ui, self.background_texture.as_ref());
        // This viewport owns its animation state, independent of the main
        // player's size and animation clock. Fish are below the glass and UI.
        if self.preferences.appearance == crate::settings::Appearance::Aqua
            && self.preferences.fish_enabled
        {
            self.equalizer_fish.paint(ui);
        } else {
            self.equalizer_fish.pause();
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(egui::Color32::TRANSPARENT)
                    .inner_margin(20),
            )
            .show(ui, |ui| {
                crate::glass_frame(ui, crate::SIDEBAR, 16, 12).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    crate::paint_frost(ui, &self.frost);
                    self.equalizer_ui(ui);
                });
            });
        crate::readability::finish_frame(ui.ctx());
    }

    pub(super) fn equalizer_ui(&mut self, ui: &mut egui::Ui) {
        let previous = self.preferences.equalizer;
        let mut settings = previous;
        ui.horizontal(|ui| {
            ui.heading("Egalizator");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::compact_icon_button(ui, "close", "Închide egalizatorul", true).clicked()
                {
                    self.equalizer_open = false;
                }
            });
        });
        ui.add_space(8.);
        ui.horizontal(|ui| {
            ui.checkbox(&mut settings.enabled, "Egalizator activ");
            ui.add_space(12.);
            ui.label("Preset");
            let selected = Preset::ALL
                .into_iter()
                .find(|preset| preset.settings() == settings)
                .map_or("Personalizat", Preset::label);
            egui::ComboBox::from_id_salt("equalizer-preset")
                .selected_text(selected)
                .width(120.)
                .show_ui(ui, |ui| {
                    for preset in Preset::ALL {
                        if ui
                            .selectable_label(selected == preset.label(), preset.label())
                            .clicked()
                        {
                            settings = preset.settings();
                        }
                    }
                });
            if ui.button("Importă JSON").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Preset egalizator", &["json"])
                    .pick_file()
                {
                    match Equalizer::load(&path) {
                        Ok(preset) => settings = preset,
                        Err(error) => self.toast(error),
                    }
                }
            }
            if ui.button("Exportă JSON").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Preset egalizator", &["json"])
                    .set_file_name("egalizator.json")
                    .save_file()
                {
                    match settings.save(&path) {
                        Ok(()) => self.toast("Preset salvat"),
                        Err(error) => self.toast(error),
                    }
                }
            }
        });
        ui.add_space(12.);
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(60., 230.),
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    ui.label(format!("{:+.1}", settings.preamp_db));
                    fader(
                        ui,
                        &mut settings.preamp_db,
                        -36.0..=12.0,
                        "Master gain / Preamp",
                    );
                    ui.label("Preamp");
                },
            );
            ui.allocate_ui_with_layout(
                Vec2::new(28., 230.),
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    ui.label(RichText::new("dB").small().color(MUTED));
                    let (axis, _) =
                        ui.allocate_exact_size(Vec2::new(28., 180.), egui::Sense::hover());
                    for (text, fraction) in [("+12", 0.), ("0", 0.5), ("−12", 1.)] {
                        let y = egui::lerp((axis.top() + 6.5)..=(axis.bottom() - 6.5), fraction);
                        ui.painter().text(
                            egui::pos2(axis.center().x, y),
                            egui::Align2::CENTER_CENTER,
                            text,
                            egui::FontId::proportional(11.),
                            MUTED,
                        );
                    }
                },
            );
            let column_width =
                ((ui.available_width() - 9. * ui.spacing().item_spacing.x) / 10.).max(42.);
            for (index, band) in settings.bands.iter_mut().enumerate() {
                ui.push_id(index, |ui| {
                    ui.allocate_ui_with_layout(
                        Vec2::new(column_width, 230.),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            ui.label(format!("{:+.1}", band.gain_db));
                            let slider = fader(
                                ui,
                                &mut band.gain_db,
                                -12.0..=12.0,
                                &format!("Banda {} · {} Hz", index + 1, band.frequency),
                            );
                            if slider.drag_started() || slider.clicked() {
                                self.equalizer_band = index;
                            }
                            if ui
                                .selectable_label(
                                    self.equalizer_band == index,
                                    frequency_label(band.frequency),
                                )
                                .clicked()
                            {
                                self.equalizer_band = index;
                            }
                        },
                    );
                });
            }
        });
        ui.add_space(8.);
        ui.horizontal(|ui| {
            if ui
                .small_button("Ajustează preamp pentru amplificări")
                .clicked()
            {
                settings.preamp_db = settings.suggested_preamp();
            }
            ui.label(
                RichText::new("Gain −12…+12 dB · Protecție de vârf activă")
                    .small()
                    .color(MUTED),
            );
        });
        self.equalizer_band = self.equalizer_band.min(settings.bands.len() - 1);
        let band = &mut settings.bands[self.equalizer_band];
        ui.add_space(8.);
        ui.horizontal(|ui| {
            ui.strong(format!("Banda {}", self.equalizer_band + 1));
            ui.label("Frecvență");
            ui.add(
                egui::DragValue::new(&mut band.frequency)
                    .range(20.0..=20000.0)
                    .speed(5.)
                    .suffix(" Hz"),
            );
            ui.label("Q");
            ui.add(
                egui::DragValue::new(&mut band.q)
                    .range(0.2..=10.0)
                    .speed(0.01)
                    .max_decimals(3),
            );
            egui::ComboBox::from_id_salt("filter-type")
                .selected_text(band.filter_type.label())
                .show_ui(ui, |ui| {
                    for kind in FilterType::ALL {
                        ui.selectable_value(&mut band.filter_type, kind, kind.label());
                    }
                });
        });
        if settings != previous {
            match self.player.set_equalizer(settings) {
                Ok(()) => {
                    self.preferences.equalizer = settings;
                    self.equalizer_save_after = Some(Instant::now() + Duration::from_millis(400));
                    ui.ctx().request_repaint_after(Duration::from_millis(400));
                }
                Err(error) => self.toast(error),
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        settings::AppSettings,
        test_support::{run_ui, TestLibrary},
    };

    #[test]
    fn startup_restores_custom_equalizer_and_drawer_exposes_its_controls() {
        let library = TestLibrary::new("equalizer-ui");
        let path = library.directory.join("settings.json");
        let settings = AppSettings {
            equalizer: Preset::Vocal.settings(),
            ..Default::default()
        };
        settings.save(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = AuraApp::with_settings_path(
            library.db.clone(),
            &ctx,
            Ok(library.music.clone()),
            Some(path),
        );
        crate::test_support::wait_for_scans(&mut app, &ctx);
        assert_eq!(app.preferences.equalizer, settings.equalizer);
        app.equalizer_open = true;
        let output = run_ui(&ctx, egui::RawInput::default(), |ui| {
            ui.set_width(780.);
            app.equalizer_ui(ui);
        });
        let text: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        for expected in [
            "Egalizator",
            "Egalizator activ",
            "Vocal",
            "Preamp",
            "Importă JSON",
            "Exportă JSON",
        ] {
            assert!(
                text.iter().any(|label| label == expected),
                "Missing {expected}: {text:?}"
            );
        }
    }

    #[test]
    fn bad_equalizer_settings_reset_only_audio_preferences() {
        let library = TestLibrary::new("equalizer-invalid-settings");
        let path = library.directory.join("settings.json");
        let mut settings = AppSettings {
            fish_enabled: false,
            ..Default::default()
        };
        settings.equalizer.bands[3].q = 0.;
        settings.save(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = AuraApp::with_settings_path(
            library.db.clone(),
            &ctx,
            Ok(library.music.clone()),
            Some(path),
        );
        crate::test_support::wait_for_scans(&mut app, &ctx);
        assert!(!app.preferences.fish_enabled);
        assert_eq!(app.preferences.equalizer, Equalizer::default());
        assert!(app
            .notification
            .as_ref()
            .unwrap()
            .0
            .contains("Egalizator resetat"));
    }

    #[test]
    fn toolbar_buttons_fit_without_overlap_with_a_narrow_drawer_layout() {
        let library = TestLibrary::new("equalizer-toolbar");
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app =
            AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
        crate::test_support::wait_for_scans(&mut app, &ctx);
        for width in [284., 640., 940.] {
            let mut bounds = egui::Rect::NOTHING;
            let output = run_ui(&ctx, egui::RawInput::default(), |ui| {
                ui.set_width(width);
                bounds = ui.max_rect();
                app.toolbar(ui, &ctx);
            });
            let tree = output.platform_output.accesskit_update.unwrap();
            let buttons: Vec<_> = tree
                .nodes
                .iter()
                .filter_map(|(_, node)| {
                    let label = node.label()?;
                    ["Importă folder", "Egalizator", "Coada"]
                        .contains(&label)
                        .then(|| (label, node.bounds().unwrap()))
                })
                .collect();
            assert_eq!(buttons.len(), 3);
            for (index, (label, rect)) in buttons.iter().enumerate() {
                assert!(
                    rect.x0 >= f64::from(bounds.left()) - 1.
                        && rect.x1 <= f64::from(bounds.right()) + 1.,
                    "{label} outside toolbar at {width}: {rect:?}"
                );
                for (other_label, other) in buttons.iter().skip(index + 1) {
                    let overlap = rect.x0 < other.x1
                        && rect.x1 > other.x0
                        && rect.y0 < other.y1
                        && rect.y1 > other.y0;
                    assert!(!overlap, "{label} overlaps {other_label} at {width}");
                }
                if *label == "Coada" {
                    assert!((rect.x1 - f64::from(bounds.right())).abs() < 2.);
                }
            }
        }
    }

    #[test]
    fn all_eleven_vertical_sliders_fit_in_the_minimum_equalizer_window() {
        let library = TestLibrary::new("equalizer-vertical");
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app =
            AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
        crate::test_support::wait_for_scans(&mut app, &ctx);
        let mut bounds = egui::Rect::NOTHING;
        let output = run_ui(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(760., 500.),
                )),
                ..Default::default()
            },
            |ui| {
                bounds = ui.max_rect();
                app.equalizer_surface(ui);
            },
        );
        let tree = output.platform_output.accesskit_update.unwrap();
        let mut sliders: Vec<_> = tree
            .nodes
            .iter()
            .filter_map(|(_, node)| {
                let label = node.label()?;
                (label == "Master gain / Preamp"
                    || label.starts_with("Banda ") && label.contains("Hz"))
                .then(|| node.bounds().unwrap())
            })
            .collect();
        sliders.sort_by(|left, right| left.x0.total_cmp(&right.x0));
        assert_eq!(sliders.len(), 11);
        for rect in &sliders {
            assert!(
                rect.y1 - rect.y0 >= 175. && rect.x1 - rect.x0 <= 30.,
                "{rect:?}"
            );
            assert!(
                rect.x0 >= f64::from(bounds.left()) && rect.x1 <= f64::from(bounds.right()) + 1.
            );
            assert!(rect.y1 <= f64::from(bounds.bottom()));
        }
        for pair in sliders.windows(2) {
            assert!(pair[0].x1 <= pair[1].x0);
        }
    }

    #[test]
    fn vertical_slider_increases_upwards_and_decreases_downwards() {
        let ctx = egui::Context::default();
        let mut value = 0.;
        let mut rect = egui::Rect::NOTHING;
        let _ = run_ui(&ctx, egui::RawInput::default(), |ui| {
            rect = fader(ui, &mut value, -12.0..=12.0, "Test band").rect;
        });
        for (y, expected) in [(rect.top() + 1., 12.), (rect.bottom() - 1., -12.)] {
            let point = egui::pos2(rect.center().x, y);
            for pressed in [true, false] {
                let _ = run_ui(
                    &ctx,
                    egui::RawInput {
                        events: vec![
                            egui::Event::PointerMoved(point),
                            egui::Event::PointerButton {
                                pos: point,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: Default::default(),
                            },
                        ],
                        ..Default::default()
                    },
                    |ui| {
                        fader(ui, &mut value, -12.0..=12.0, "Test band");
                    },
                );
            }
            assert!((value - expected).abs() < 0.01);
        }
    }

    #[test]
    fn theme_photo_covers_the_equalizer_and_is_painted_before_every_control() {
        let library = TestLibrary::new("equalizer-photo");
        let path = library.directory.join("photo.png");
        image::RgbaImage::from_pixel(32, 20, image::Rgba([220, 230, 240, 255]))
            .save(&path)
            .unwrap();
        let ctx = egui::Context::default();
        let mut app =
            AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
        crate::test_support::wait_for_scans(&mut app, &ctx);
        for appearance in [
            crate::settings::Appearance::Aqua,
            crate::settings::Appearance::Beer,
            crate::settings::Appearance::Disco,
        ] {
            app.preferences.appearance = appearance;
            app.preferences.set_background(Some(path.clone()));
            app.reload_appearance(&ctx);
            let texture_id = app.background_texture.as_ref().unwrap().id();
            let mut viewport = egui::Rect::NOTHING;
            let output = run_ui(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(820., 500.),
                    )),
                    ..Default::default()
                },
                |ui| {
                    viewport = ui.max_rect();
                    app.equalizer_surface(ui);
                },
            );
            let photo = output.shapes.iter().position(|shape| matches!(&shape.shape, egui::Shape::Mesh(mesh) if mesh.texture_id == texture_id)).unwrap();
            let first_text = output
                .shapes
                .iter()
                .position(|shape| matches!(&shape.shape, egui::Shape::Text(_)))
                .unwrap();
            assert!(
                photo < first_text,
                "{appearance:?}: background painted over controls"
            );
            if let egui::Shape::Mesh(mesh) = &output.shapes[photo].shape {
                let bounds = egui::Rect::from_points(
                    &mesh
                        .vertices
                        .iter()
                        .map(|vertex| vertex.pos)
                        .collect::<Vec<_>>(),
                );
                assert!(bounds.contains_rect(viewport));
            }
            assert!(output.shapes.iter().skip(first_text).all(|shape| !matches!(&shape.shape, egui::Shape::Mesh(mesh) if mesh.texture_id == texture_id)));
        }
    }

    #[test]
    fn one_glass_panel_contains_all_controls_and_aqua_fish_swim_below_it() {
        fn flatten<'a>(
            index: usize,
            shape: &'a egui::Shape,
            output: &mut Vec<(usize, &'a egui::Shape)>,
        ) {
            if let egui::Shape::Vec(children) = shape {
                for child in children {
                    flatten(index, child, output);
                }
            } else {
                output.push((index, shape));
            }
        }
        let library = TestLibrary::new("equalizer-glass-fish");
        let ctx = egui::Context::default();
        let mut app =
            AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
        crate::test_support::wait_for_scans(&mut app, &ctx);
        let mut previous_fish = Vec::new();
        for (time, appearance, enabled, expected_fish) in [
            (0.0, crate::settings::Appearance::Aqua, true, 2),
            (0.1, crate::settings::Appearance::Aqua, true, 2),
            (0.2, crate::settings::Appearance::Aqua, false, 0),
            (0.3, crate::settings::Appearance::Beer, true, 0),
            (0.4, crate::settings::Appearance::Disco, true, 0),
        ] {
            if app.preferences.appearance != appearance {
                app.preferences.appearance = appearance;
                app.reload_appearance(&ctx);
            }
            app.preferences.fish_enabled = enabled;
            let photo_id = app.background_texture.as_ref().map(|texture| texture.id());
            let frost_id = app.frost.id();
            let output = run_ui(
                &ctx,
                egui::RawInput {
                    time: Some(time),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(760., 500.),
                    )),
                    ..Default::default()
                },
                |ui| app.equalizer_surface(ui),
            );
            let glass_color = crate::theme::current(&ctx).sidebar;
            let mut shapes = Vec::new();
            for (index, shape) in output.shapes.iter().enumerate() {
                flatten(index, &shape.shape, &mut shapes);
            }
            let panes: Vec<_> = shapes
                .iter()
                .filter_map(|(index, shape)| match shape {
                    egui::Shape::Rect(rect) if rect.fill == glass_color => {
                        Some((*index, rect.rect))
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                panes.len(),
                1,
                "{appearance:?}: expected one shared glass panel"
            );
            let (pane_index, pane) = panes[0];
            assert!(
                egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(760., 500.))
                    .contains_rect(pane),
                "Panel exceeds minimum window: {pane:?}"
            );
            for (index, shape) in &shapes {
                if let egui::Shape::Text(text) = shape {
                    assert!(*index > pane_index);
                    assert!(
                        pane.contains_rect(egui::Rect::from_min_size(text.pos, text.galley.size())),
                        "Text outside shared glass: {}",
                        text.galley.text()
                    );
                }
            }
            let fish: Vec<_> = shapes
                .iter()
                .filter_map(|(index, shape)| match shape {
                    egui::Shape::Mesh(mesh)
                        if mesh.texture_id != egui::TextureId::default()
                            && Some(mesh.texture_id) != photo_id
                            && mesh.texture_id != frost_id =>
                    {
                        assert!(*index < pane_index, "Fish painted over controls");
                        Some(
                            mesh.vertices
                                .iter()
                                .map(|vertex| vertex.pos)
                                .collect::<Vec<_>>(),
                        )
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(fish.len(), expected_fish);
            if time == 0.1 {
                assert_ne!(fish, previous_fish, "Fish did not move between frames");
            }
            previous_fish = fish;
        }
    }
}
