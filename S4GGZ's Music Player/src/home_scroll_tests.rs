use super::*;

fn leaves<'a>(shape: &'a egui::Shape, out: &mut Vec<&'a egui::Shape>) {
    if let egui::Shape::Vec(shapes) = shape {
        for shape in shapes {
            leaves(shape, out);
        }
    } else {
        out.push(shape);
    }
}

fn shapes(output: &egui::FullOutput) -> Vec<&egui::Shape> {
    let mut shapes = Vec::new();
    for entry in &output.shapes {
        leaves(&entry.shape, &mut shapes);
    }
    shapes
}

fn text_y(output: &egui::FullOutput, label: &str) -> f32 {
    shapes(output)
        .into_iter()
        .find_map(|shape| match shape {
            egui::Shape::Text(text) if text.galley.text() == label => Some(text.pos.y),
            _ => None,
        })
        .unwrap_or_else(|| panic!("Visible text missing: {label}"))
}

fn fill_albums(app: &mut AuraApp, path: &std::path::Path, count: usize) {
    app.albums = (0..count)
        .map(|index| Album {
            id: index as i64 + 1,
            title: format!("Album {index:04}"),
            artist: "QA Artist".into(),
            track_count: 1,
            duration_ms: 1000,
            representative_track: 1,
            representative_path: path.to_string_lossy().into_owned(),
        })
        .collect();
}

fn render(
    app: &mut AuraApp,
    ctx: &egui::Context,
    size: Vec2,
    events: Vec<egui::Event>,
) -> (egui::FullOutput, egui::scroll_area::ScrollAreaOutput<()>) {
    let mut page = None;
    let output = crate::test_support::run_ui(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ui| {
            paint_ambient(ui, app.background_texture.as_ref());
            app.disco.set_stage_bounds(egui::Rect::from_x_y_ranges(
                263.0..=size.x - if app.drawer.is_some() { 344.0 } else { 24.0 },
                0.0..=size.y,
            ));
            app.disco.begin_frame(
                ui,
                app.preferences.disco_mode,
                true,
                false,
                Default::default(),
            );
            egui::Panel::bottom("test-footer")
                .exact_size(126.0)
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    glass_frame(ui, SIDEBAR, 18, 12).show(ui, |ui| {
                        ui.label("Player");
                    });
                });
            egui::Panel::left("test-sidebar")
                .exact_size(239.0)
                .frame(glass_frame(ui, SIDEBAR, 18, 15))
                .show(ui, |ui| app.sidebar(ui, ctx));
            if app.drawer.is_some() {
                egui::Panel::right("test-drawer")
                    .exact_size(320.0)
                    .frame(glass_frame(ui, SIDEBAR, 18, 18))
                    .show(ui, |ui| {
                        ui.label("Drawer");
                    });
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.inner_margin(24))
                .show(ui, |ui| {
                    page = Some(app.home(ui, ctx));
                });
            app.disco.finish_frame(ctx);
        },
    );
    (output, page.unwrap())
}

fn scroll_to(ctx: &egui::Context, page: &egui::scroll_area::ScrollAreaOutput<()>, offset: f32) {
    let mut state = page.state;
    state.offset.y = offset.clamp(
        0.0,
        (page.content_size.y - page.inner_rect.height()).max(0.0),
    );
    state.store(ctx, page.id);
}

fn assert_ball_under_menus(output: &egui::FullOutput, ctx: &egui::Context) -> egui::Pos2 {
    let shapes = shapes(output);
    let (ball_index, center) = shapes
        .iter()
        .enumerate()
        .find_map(|(index, shape)| match shape {
            egui::Shape::Circle(circle) if circle.fill == Color32::from_rgb(34, 35, 49) => {
                Some((index, circle.center))
            }
            _ => None,
        })
        .expect("Disco ball missing");
    let palette = theme::current(ctx);
    let menus: Vec<_> = shapes
        .iter()
        .enumerate()
        .filter_map(|(index, shape)| match shape {
            egui::Shape::Rect(rect) if [palette.card, palette.sidebar].contains(&rect.fill) => {
                Some((index, rect.rect))
            }
            _ => None,
        })
        .collect();
    assert!(menus.len() >= 5);
    assert!(
        menus.iter().all(|(index, _)| *index > ball_index),
        "A menu was painted below the ball"
    );
    center
}

#[test]
fn all_home_panels_scroll_together_while_the_ball_stays_behind_them() {
    let library = TestLibrary::new("home-panels-scroll");
    let path = library.mp3("QA Artist", "silence");
    let ctx = egui::Context::default();
    let mut app =
        AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
    wait_for_scans(&mut app, &ctx);
    app.preferences.appearance = Appearance::Disco;
    app.reload_appearance(&ctx);
    app.disco.set_ray_count(0);
    fill_albums(&mut app, &path, 60);
    let size = Vec2::new(1280.0, 790.0);
    // Settle scrollbar width before comparing positions.
    render(&mut app, &ctx, size, vec![]);
    let (before, page) = render(&mut app, &ctx, size, vec![]);
    let center = assert_ball_under_menus(&before, &ctx);
    scroll_to(&ctx, &page, 20.0);
    let (after, page) = render(&mut app, &ctx, size, vec![]);
    assert!((page.state.offset.y - 20.0).abs() < 0.1);
    for label in [
        "Importă folder",
        "Redescoperă muzica ta.",
        "MELODII",
        "Din colecția ta",
    ] {
        assert!(
            (text_y(&before, label) - text_y(&after, label) - 20.0).abs() < 0.2,
            "{label} did not scroll with Home"
        );
    }
    scroll_to(&ctx, &page, 120.0);
    let (overlap, _) = render(&mut app, &ctx, size, vec![]);
    assert_eq!(assert_ball_under_menus(&overlap, &ctx), center);
    let palette = theme::current(&ctx);
    assert!(shapes(&overlap).iter().any(|shape| matches!(shape, egui::Shape::Rect(rect) if rect.fill == palette.card && rect.rect.contains(center))), "Test must put a scrolled menu over the ball");
    app.drawer = Some(Drawer::Queue);
    let (resized, _) = render(&mut app, &ctx, Vec2::new(950.0, 610.0), vec![]);
    assert_ball_under_menus(&resized, &ctx);
}

#[test]
fn home_reaches_the_last_album_without_rendering_the_entire_library() {
    let library = TestLibrary::new("home-large-grid");
    let path = library.mp3("QA Artist", "silence");
    let ctx = egui::Context::default();
    let mut app =
        AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
    wait_for_scans(&mut app, &ctx);
    app.disco.set_ray_count(0);
    fill_albums(&mut app, &path, 2000);
    for size in [Vec2::new(1280.0, 790.0), Vec2::new(950.0, 610.0)] {
        let (_, page) = render(&mut app, &ctx, size, vec![]);
        assert!(page.content_size.y > page.inner_rect.height() + 1000.0);
        scroll_to(&ctx, &page, page.content_size.y);
        let (bottom, page) = render(&mut app, &ctx, size, vec![]);
        assert!(page.state.offset.y + page.inner_rect.height() >= page.content_size.y - 2.0);
        assert!(text_y(&bottom, "Album 1999") < page.inner_rect.bottom());
        let drawn = shapes(&bottom).iter().filter(|shape| matches!(shape, egui::Shape::Text(text) if text.galley.text().starts_with("Album "))).count();
        assert!(
            (1..=15).contains(&drawn),
            "Expected visible rows, got {drawn} album labels"
        );
        assert_eq!(app.albums.len(), 2000);
        if size.x == 950.0 {
            let title = shapes(&bottom)
                .into_iter()
                .find_map(|shape| match shape {
                    egui::Shape::Text(text) if text.galley.text() == "Album 1999" => Some(text.pos),
                    _ => None,
                })
                .unwrap();
            let play = shapes(&bottom)
                .into_iter()
                .find_map(|shape| match shape {
                    egui::Shape::Text(text)
                        if text.galley.text().starts_with('▶')
                            && text.pos.x >= title.x
                            && text.pos.x < title.x + ALBUM_CARD_SIZE - 24.0
                            && text.pos.y > title.y
                            && text.pos.y < title.y + 60.0 =>
                    {
                        Some(text.pos + text.galley.rect.center().to_vec2())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| {
                    let nearby: Vec<_> = shapes(&bottom).into_iter().filter_map(|shape| match shape {
                        egui::Shape::Text(text) if (text.pos.y-title.y).abs() < 100.0 => Some((text.galley.text(), text.pos)),
                        _ => None,
                    }).collect();
                    panic!("Last album play missing; title={title:?}, viewport={:?}, offset={:?}, nearby={nearby:?}", page.inner_rect, page.state.offset);
                });
            for pressed in [true, false] {
                render(
                    &mut app,
                    &ctx,
                    size,
                    vec![
                        egui::Event::PointerMoved(play),
                        egui::Event::PointerButton {
                            pos: play,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            assert!(app.screen == Screen::Songs);
            assert_eq!(app.album_filter, Some(2000));
            assert_eq!(app.album_filter_title, "Album 1999");
            wait_for_scans(&mut app, &ctx);
        }
    }
}

#[test]
fn home_empty_library_keeps_the_import_action_reachable() {
    let library = TestLibrary::new("home-empty-scroll");
    let ctx = egui::Context::default();
    let mut app =
        AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
    wait_for_scans(&mut app, &ctx);
    app.preferences.appearance = Appearance::Disco;
    app.reload_appearance(&ctx);
    app.disco.set_ray_count(0);
    let size = Vec2::new(950.0, 610.0);
    let (_, page) = render(&mut app, &ctx, size, vec![]);
    scroll_to(&ctx, &page, page.content_size.y);
    let (bottom, page) = render(&mut app, &ctx, size, vec![]);
    let y = text_y(&bottom, "Alege un folder");
    assert!(y >= page.inner_rect.top() && y < page.inner_rect.bottom());
}

#[test]
fn mouse_wheel_over_home_panels_and_albums_uses_the_same_scroll_position() {
    let library = TestLibrary::new("home-wheel");
    let path = library.mp3("QA Artist", "silence");
    let ctx = egui::Context::default();
    let mut app =
        AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
    wait_for_scans(&mut app, &ctx);
    app.disco.set_ray_count(0);
    fill_albums(&mut app, &path, 60);
    let size = Vec2::new(1280.0, 790.0);
    let (_, mut page) = render(&mut app, &ctx, size, vec![]);
    for offset in [0.0, 500.0] {
        scroll_to(&ctx, &page, offset);
        let (_, settled) = render(&mut app, &ctx, size, vec![]);
        let pointer = egui::Pos2::new(
            settled.inner_rect.center().x,
            settled.inner_rect.top() + 180.0,
        );
        let (_, moved) = render(
            &mut app,
            &ctx,
            size,
            vec![
                egui::Event::PointerMoved(pointer),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: Vec2::new(0.0, -180.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert_eq!(moved.id, settled.id);
        assert!(
            moved.state.offset.y > offset,
            "Wheel did not scroll Home at offset {offset}"
        );
        page = moved;
    }
}
