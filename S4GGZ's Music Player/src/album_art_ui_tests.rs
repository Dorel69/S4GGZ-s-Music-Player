use crate::test_support::{run_ui, wait_for_scans, TestLibrary};
use crate::*;

fn render(
    app: &mut AuraApp,
    ctx: &egui::Context,
    album: &Album,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    run_ui(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(420.0, 340.0),
            )),
            events,
            ..Default::default()
        },
        |ui| app.album_cards(ui, ctx, std::slice::from_ref(album), None),
    )
}

fn has_button(output: &egui::FullOutput, name: &str) -> bool {
    output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .any(|(_, node)| node.label() == Some(name))
}

#[test]
fn album_click_offers_add_then_change_and_custom_picture_reaches_the_footer_cache() {
    let library = TestLibrary::new("album-picture-menu");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app =
        AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
    wait_for_scans(&mut app, &ctx);
    let album = Album {
        id: 71,
        title: "My album".into(),
        artist: "Artist".into(),
        track_count: 1,
        duration_ms: 1000,
        representative_track: 1,
        representative_path: library
            .music
            .join("missing.mp3")
            .to_string_lossy()
            .into_owned(),
    };
    let output = render(&mut app, &ctx, &album, vec![]);
    let bounds = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Opțiuni album My album"))
        .unwrap()
        .1
        .bounds()
        .unwrap();
    let point = egui::pos2(bounds.x0 as f32 + 45.0, bounds.y0 as f32 + 45.0);
    for pressed in [true, false] {
        render(
            &mut app,
            &ctx,
            &album,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    let output = render(&mut app, &ctx, &album, vec![]);
    assert!(has_button(&output, "Redenumește albumul"));
    let outside = egui::pos2(415.0, 335.0);
    render(
        &mut app,
        &ctx,
        &album,
        vec![
            egui::Event::PointerMoved(outside),
            egui::Event::PointerButton {
                pos: outside,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos: outside,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    for pressed in [true, false] {
        render(
            &mut app,
            &ctx,
            &album,
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
    let output = render(&mut app, &ctx, &album, vec![]);
    assert!(
        has_button(&output, "Add picture"),
        "Clicking the picture must expose the menu"
    );
    let source = library.directory.join("source.png");
    image::RgbaImage::from_pixel(32, 24, image::Rgba([10, 200, 40, 255]))
        .save(&source)
        .unwrap();
    let directory = app.album_art.as_ref().unwrap().directory().to_path_buf();
    let name = album_art::import(&directory, album.id, &source).unwrap();
    app.tx
        .send(UiEvent::AlbumPictureChosen {
            id: album.id,
            result: Ok(Some(name)),
        })
        .unwrap();
    app.poll_events(&ctx);
    std::fs::remove_file(source).unwrap();
    let output = render(&mut app, &ctx, &album, vec![]);
    assert!(has_button(&output, "Change picture"));
    let deadline = Instant::now() + Duration::from_secs(3);
    while app.album_cover(album.id, &ctx).is_none() {
        assert!(
            Instant::now() < deadline,
            "Album picture must load asynchronously"
        );
        app.poll_events(&ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(album_art::Store::open(directory)
        .unwrap()
        .path(album.id)
        .is_some());
}
