use crate::test_support::{run_ui, wait_for_scans, TestLibrary};
use crate::*;

fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn accessible_bounds(output: &egui::FullOutput, label: &str) -> egui::Rect {
    output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find_map(|(_, node)| {
            (node.label().or_else(|| node.value()) == Some(label)).then(|| {
                let bounds = node.bounds().unwrap();
                egui::Rect::from_min_max(
                    egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
                    egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
                )
            })
        })
        .unwrap_or_else(|| panic!("Missing accessible control: {label}"))
}

fn footer_frame(
    app: &mut AuraApp,
    ctx: &egui::Context,
    width: f32,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let status = app.player.status().unwrap();
    run_ui(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(width, 120.0),
            )),
            events,
            ..Default::default()
        },
        |ui| app.footer(ui, ctx, &status),
    )
}

fn click_footer_control(app: &mut AuraApp, ctx: &egui::Context, label: &str) {
    let output = footer_frame(app, ctx, 894.0, vec![]);
    let point = accessible_bounds(&output, label).center();
    for pressed in [true, false] {
        footer_frame(
            app,
            ctx,
            894.0,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
}

fn wait_for_mode(app: &AuraApp, mode: player::PlaybackMode) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let status = app.player.status().unwrap();
        if status.playback_mode == mode {
            assert!(status.error.is_none());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Playback mode command was not applied"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn press_key(app: &mut AuraApp, ctx: &egui::Context, pressed_key: egui::Key) {
    footer_frame(app, ctx, 894.0, vec![key(pressed_key)]);
    let mut released = key(pressed_key);
    if let egui::Event::Key { pressed, .. } = &mut released {
        *pressed = false;
    }
    footer_frame(app, ctx, 894.0, vec![released]);
}

#[test]
fn footer_modes_fit_and_swap_at_the_minimum_width_in_every_theme() {
    let library = TestLibrary::new("exclusive-responsive-footer");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app =
        AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
    wait_for_scans(&mut app, &ctx);
    for appearance in [Appearance::Aqua, Appearance::Beer, Appearance::Disco] {
        theme::install(&ctx, GlassPalette::for_appearance(appearance));
        for mode in [player::PlaybackMode::Shuffle, player::PlaybackMode::Loop] {
            app.player.set_playback_mode(mode).unwrap();
            wait_for_mode(&app, mode);
            for width in [894.0, 1280.0] {
                let output = footer_frame(&mut app, &ctx, width, vec![]);
                let shuffle = accessible_bounds(&output, "Shuffle");
                let loop_button = accessible_bounds(&output, "Loop");
                let controls = [
                    ("Volume", accessible_bounds(&output, "Volume")),
                    (
                        "Piesa anterioară",
                        accessible_bounds(&output, "Piesa anterioară"),
                    ),
                    ("Play", accessible_bounds(&output, "Play")),
                    ("Shuffle/Loop", shuffle.union(loop_button)),
                    (
                        "Următoarea piesă",
                        accessible_bounds(&output, "Următoarea piesă"),
                    ),
                    (
                        "Poziție redare",
                        accessible_bounds(&output, "Poziție redare"),
                    ),
                    (
                        "Coada de redare",
                        accessible_bounds(&output, "Coada de redare"),
                    ),
                ];
                for (label, rect) in &controls {
                    assert!(
                        rect.left() >= 0.0 && rect.right() <= width + 1.0,
                        "{appearance:?}, {mode:?}, width {width}: {label} clipped: {rect:?}"
                    );
                    assert!(rect.top() >= 0.0 && rect.bottom() <= 120.0);
                }
                for pair in controls.windows(2) {
                    assert!(
                        pair[0].1.right() <= pair[1].1.left(),
                        "{appearance:?}, {mode:?}, width {width}: {} overlaps {}",
                        pair[0].0,
                        pair[1].0
                    );
                }
                let (small, large) = match mode {
                    player::PlaybackMode::Shuffle => (loop_button, shuffle),
                    player::PlaybackMode::Loop => (shuffle, loop_button),
                    player::PlaybackMode::Off => unreachable!(),
                };
                assert!(small.width() < large.width() && small.height() < large.height());
                assert!(small.center().x > large.center().x && small.center().y < large.center().y);
                assert!(accessible_bounds(&output, "Poziție redare").width() >= 80.0);
            }
        }
    }
}

fn render_sidebar(
    app: &mut AuraApp,
    ctx: &egui::Context,
    size: Vec2,
) -> (
    egui::FullOutput,
    egui::Rect,
    egui::scroll_area::ScrollAreaOutput<()>,
) {
    let mut sidebar = egui::Rect::NOTHING;
    let mut navigation = None;
    let output = run_ui(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        },
        |ui| {
            egui::Panel::bottom("short-sidebar-footer")
                .exact_size(126.0)
                .frame(egui::Frame::NONE)
                .show(ui, |_| {});
            egui::Panel::left("short-sidebar-panel")
                .exact_size(239.0)
                .resizable(false)
                .frame(glass_frame(ui, SIDEBAR, 18, 15).outer_margin(egui::Margin {
                    left: 12,
                    right: 4,
                    top: 12,
                    bottom: 4,
                }))
                .show(ui, |ui| {
                    sidebar = ui.clip_rect();
                    navigation = Some(app.sidebar(ui, ctx));
                });
        },
    );
    (output, sidebar, navigation.unwrap())
}

#[test]
fn sidebar_controls_stay_reachable_at_short_heights() {
    let library = TestLibrary::new("short-sidebar");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app =
        AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
    wait_for_scans(&mut app, &ctx);

    let controls = [
        "Acasă",
        "Toate piesele",
        "Albume",
        "Playlisturi",
        "Favorite",
        "Adaugă muzică",
        "Descarcă melodii",
        "Rescanează",
        "Setări",
    ];
    let fully_inside = |rect: egui::Rect, area: egui::Rect| {
        rect.left() >= area.left() - 1.0
            && rect.right() <= area.right() + 1.0
            && rect.top() >= area.top() - 1.0
            && rect.bottom() <= area.bottom() + 1.0
    };

    for size in [
        Vec2::new(950.0, 610.0),
        Vec2::new(1280.0, 790.0),
        Vec2::new(1920.0, 1080.0),
    ] {
        let (_, _, first_navigation) = render_sidebar(&mut app, &ctx, size);
        let mut state = first_navigation.state;
        state.offset.y = 0.0;
        state.store(&ctx, first_navigation.id);
        let (output, panel, navigation) = render_sidebar(&mut app, &ctx, size);
        assert!(fully_inside(accessible_bounds(&output, "Setări"), panel));
        assert!(
            navigation.inner_rect.bottom() <= accessible_bounds(&output, "Setări").top(),
            "The pinned settings button overlaps the scrollable navigation at {size:?}"
        );

        let max_offset = (navigation.content_size.y - navigation.inner_rect.height()).max(0.0);
        if size.y <= 610.0 {
            assert!(max_offset > 0.0, "short sidebar must become scrollable");
            for label in [
                "Acasă",
                "Toate piesele",
                "Albume",
                "Playlisturi",
                "Favorite",
            ] {
                assert!(
                    fully_inside(accessible_bounds(&output, label), navigation.inner_rect),
                    "{label} is not visible at the top of the compact sidebar"
                );
            }
            state = navigation.state;
            state.offset.y = max_offset;
            state.store(&ctx, navigation.id);
            let (scrolled, panel, navigation) = render_sidebar(&mut app, &ctx, size);
            for label in ["Adaugă muzică", "Descarcă melodii", "Rescanează"] {
                assert!(
                    fully_inside(accessible_bounds(&scrolled, label), navigation.inner_rect),
                    "{label} cannot be brought into view in the compact sidebar"
                );
            }
            assert!(fully_inside(accessible_bounds(&scrolled, "Setări"), panel));
        } else {
            for label in controls {
                assert!(
                    fully_inside(accessible_bounds(&output, label), panel),
                    "{label} is clipped at {size:?}"
                );
            }
        }
    }
}

#[test]
fn active_modes_toggle_off_and_preserve_the_last_primary_position() {
    let library = TestLibrary::new("exclusive-mode-input");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app =
        AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
    wait_for_scans(&mut app, &ctx);
    wait_for_mode(&app, player::PlaybackMode::Off);

    click_footer_control(&mut app, &ctx, "Loop");
    wait_for_mode(&app, player::PlaybackMode::Loop);
    let output = footer_frame(&mut app, &ctx, 894.0, vec![]);
    assert!(
        accessible_bounds(&output, "Loop").width() > accessible_bounds(&output, "Shuffle").width()
    );
    assert!(
        ctx.egui_wants_keyboard_input(),
        "Promoted mode must retain focus"
    );

    // A second click disables the active mode without moving either control.
    click_footer_control(&mut app, &ctx, "Loop");
    wait_for_mode(&app, player::PlaybackMode::Off);
    let output = footer_frame(&mut app, &ctx, 894.0, vec![]);
    assert!(
        accessible_bounds(&output, "Loop").width() > accessible_bounds(&output, "Shuffle").width()
    );
    press_key(&mut app, &ctx, egui::Key::Space);
    wait_for_mode(&app, player::PlaybackMode::Loop);

    click_footer_control(&mut app, &ctx, "Shuffle");
    wait_for_mode(&app, player::PlaybackMode::Shuffle);
    let output = footer_frame(&mut app, &ctx, 894.0, vec![]);
    assert!(
        accessible_bounds(&output, "Shuffle").width() > accessible_bounds(&output, "Loop").width()
    );
    press_key(&mut app, &ctx, egui::Key::Space);
    wait_for_mode(&app, player::PlaybackMode::Off);
    press_key(&mut app, &ctx, egui::Key::Space);
    wait_for_mode(&app, player::PlaybackMode::Shuffle);

    // Tab moves from the selected main control to the small alternative.
    footer_frame(&mut app, &ctx, 894.0, vec![]);
    press_key(&mut app, &ctx, egui::Key::Tab);
    press_key(&mut app, &ctx, egui::Key::Space);
    wait_for_mode(&app, player::PlaybackMode::Loop);
}

#[test]
fn playback_modes_expose_two_optional_exclusive_theme_controls() {
    fn has_accent(shape: &egui::Shape, accent: Color32) -> bool {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().any(|shape| has_accent(shape, accent)),
            egui::Shape::Path(path) => {
                path.stroke.color == egui::epaint::ColorMode::Solid(accent) || path.fill == accent
            }
            egui::Shape::Circle(circle) => circle.fill == accent,
            _ => false,
        }
    }
    fn has_single_track_badge(shape: &egui::Shape) -> bool {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().any(has_single_track_badge),
            egui::Shape::Text(text) => text.galley.text() == "1",
            _ => false,
        }
    }
    for appearance in [Appearance::Aqua, Appearance::Beer, Appearance::Disco] {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx, GlassPalette::for_appearance(appearance));
        for mode in [
            player::PlaybackMode::Shuffle,
            player::PlaybackMode::Loop,
            player::PlaybackMode::Off,
        ] {
            let mut primary = player::PlaybackMode::Loop;
            let output = run_ui(&ctx, egui::RawInput::default(), |ui| {
                let _ = playback_modes::show(ui, mode, &mut primary);
            });
            let tree = output.platform_output.accesskit_update.as_ref().unwrap();
            let modes: Vec<_> = tree
                .nodes
                .iter()
                .filter(|(_, node)| node.role() == egui::accesskit::Role::CheckBox)
                .collect();
            assert_eq!(modes.len(), 2);
            for (label, selected) in [
                ("Shuffle", mode == player::PlaybackMode::Shuffle),
                ("Loop", mode == player::PlaybackMode::Loop),
            ] {
                let node = modes
                    .iter()
                    .find(|(_, node)| node.label() == Some(label))
                    .unwrap();
                // egui response.rs maps WidgetInfo.selected to AccessKit.toggled,
                // including for Role::RadioButton (not AccessKit.selected).
                assert_eq!(
                    node.1.toggled(),
                    Some(if selected {
                        egui::accesskit::Toggled::True
                    } else {
                        egui::accesskit::Toggled::False
                    })
                );
            }
            assert_eq!(
                output
                    .shapes
                    .iter()
                    .any(|shape| has_accent(&shape.shape, theme::accent(&ctx))),
                mode != player::PlaybackMode::Off
            );
            assert!(!output
                .shapes
                .iter()
                .any(|shape| has_single_track_badge(&shape.shape)));
        }
    }
}

#[test]
fn download_queue_is_a_right_panel_only_on_download_at_compact_and_regular_widths() {
    let library = TestLibrary::new("download-panel-layout");
    for appearance in [Appearance::Aqua, Appearance::Beer, Appearance::Disco] {
        for width in [950.0, 1280.0] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let mut app =
                AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
            wait_for_scans(&mut app, &ctx);
            app.preferences.appearance = appearance;
            app.preferences.disco_ball_enabled = false;
            theme::install(&ctx, GlassPalette::for_appearance(appearance));

            for screen in [Screen::AddSongs, Screen::Home, Screen::Favorites] {
                app.screen = screen;
                let status = app.player.status().unwrap();
                let mut content_bounds = egui::Rect::NOTHING;
                let output = run_ui(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            Vec2::new(width, 610.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        egui::Panel::bottom("download-layout-footer")
                            .exact_size(126.0)
                            .frame(egui::Frame::NONE)
                            .show(ui, |_| {});
                        egui::Panel::left("download-layout-sidebar")
                            .exact_size(239.0)
                            .frame(egui::Frame::NONE)
                            .show(ui, |_| {});
                        app.right_panels(ui, &ctx, &status);
                        egui::CentralPanel::default()
                            .frame(egui::Frame::NONE.inner_margin(24))
                            .show(ui, |ui| {
                                content_bounds = ui.max_rect();
                                app.main_content(ui, &ctx);
                            });
                    },
                );
                // egui labels use AccessKit's value field; interactive controls
                // use label. Inspect both so this checks the actual heading.
                let visible_text: Vec<_> = output
                    .platform_output
                    .accesskit_update
                    .as_ref()
                    .unwrap()
                    .nodes
                    .iter()
                    .filter_map(|(_, node)| node.label().or_else(|| node.value()))
                    .collect();
                let queue_visible = visible_text.contains(&"Coada de descărcări");
                assert_eq!(
                    queue_visible,
                    screen == Screen::AddSongs,
                    "{appearance:?}, width {width}, download={}: visible text={visible_text:?}",
                    screen == Screen::AddSongs
                );
                if queue_visible {
                    let heading = accessible_bounds(&output, "Coada de descărcări");
                    assert!(
                        heading.left() >= content_bounds.right(),
                        "{appearance:?}, {width}: queue must be right of the search form"
                    );
                    assert!(
                        heading.right() <= width
                            && heading.top() >= 0.0
                            && heading.bottom() < 610.0 - 126.0,
                        "{appearance:?}, {width}: queue header must stay inside its panel"
                    );
                    assert!(
                        content_bounds.width() >= 360.0,
                        "Search form must retain usable width"
                    );
                }
            }
        }
    }
}
