use super::*;
use crate::test_support::{run_ui, TestLibrary};
use std::time::Instant;

type Snapshot = (String, String, String);

fn controlled() -> (
    Downloader,
    Receiver<Snapshot>,
    Sender<Result<String, String>>,
) {
    let (started_tx, started_rx) = mpsc::channel();
    let (complete_tx, complete_rx) = mpsc::channel();
    let complete_rx = Arc::new(Mutex::new(complete_rx));
    let mut downloader = Downloader::new();
    downloader.executor = Arc::new(
        move |url, title, artist, _options, _db, _progress, cancel| {
            started_tx
                .send((url.into(), title.into(), artist.into()))
                .unwrap();
            loop {
                process_runner::check_cancel(cancel)?;
                match complete_rx
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_millis(5))
                {
                    Ok(result) => return result,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        return Err("Test worker disconnected".into())
                    }
                }
            }
        },
    );
    (downloader, started_rx, complete_tx)
}

fn finish(downloader: &mut Downloader) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(result) = downloader.poll() {
            return result;
        }
        assert!(
            Instant::now() < deadline,
            "Worker completion did not arrive"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn enqueue(downloader: &mut Downloader, library: &TestLibrary, ctx: &egui::Context, name: &str) {
    let mut url = reqwest::Url::parse("https://example.com/").unwrap();
    url.path_segments_mut().unwrap().push(name);
    assert!(downloader.start_download_url(
        url.to_string(),
        name.into(),
        "Artist".into(),
        library.db.clone(),
        ctx
    ));
}

#[test]
fn playlist_download_mode_keeps_album_and_playlist_choices_distinct() {
    let library = TestLibrary::new("playlist-save-modes");
    let ctx = egui::Context::default();
    let (mut downloader, started, complete) = controlled();
    assert_eq!(downloader.playlist_save_mode, PlaylistSaveMode::Album);

    downloader.playlist_context =
        Some((library.db.clone(), ctx.clone(), PlaylistSaveMode::Playlist));
    downloader
        .playlist_tx
        .send(Ok(crate::music_search::PlaylistResult {
            title: "Road Mix".into(),
            tracks: vec![SearchResult {
                title: "First Song".into(),
                artist: "Artist".into(),
                duration: "3:00".into(),
                page_url: "https://example.com/first".into(),
                download_url: Some("https://example.com/first".into()),
            }],
        }))
        .unwrap();
    downloader.poll();

    let playlist_id = library.db.playlists().unwrap()[0].id;
    let job = downloader.active.as_ref().unwrap();
    assert_eq!(job.options.playlist_album, None);
    assert_eq!(job.options.playlist_id, Some(playlist_id));
    assert_eq!(
        started.recv_timeout(Duration::from_secs(1)).unwrap().0,
        "https://example.com/first"
    );
    complete.send(Ok("done".into())).unwrap();
    assert_eq!(finish(&mut downloader).unwrap(), "done");

    let (mut album_downloader, album_started, album_complete) = controlled();
    album_downloader.playlist_context = Some((library.db.clone(), ctx, PlaylistSaveMode::Album));
    album_downloader
        .playlist_tx
        .send(Ok(crate::music_search::PlaylistResult {
            title: "Night Mix".into(),
            tracks: vec![SearchResult {
                title: "Second Song".into(),
                artist: "Artist".into(),
                duration: "3:00".into(),
                page_url: "https://example.com/second".into(),
                download_url: Some("https://example.com/second".into()),
            }],
        }))
        .unwrap();
    album_downloader.poll();
    let album_job = album_downloader.active.as_ref().unwrap();
    assert_eq!(
        album_job.options.playlist_album.as_deref(),
        Some("Night Mix")
    );
    assert_eq!(album_job.options.playlist_id, None);
    assert_eq!(library.db.playlists().unwrap().len(), 1);
    album_started.recv_timeout(Duration::from_secs(1)).unwrap();
    album_complete.send(Ok("done".into())).unwrap();
    assert_eq!(finish(&mut album_downloader).unwrap(), "done");
}

#[test]
fn fifo_queue_keeps_request_snapshots_deduplicates_and_preserves_current_progress() {
    let library = TestLibrary::new("download-fifo");
    let ctx = egui::Context::default();
    let (mut downloader, started, complete) = controlled();
    downloader.url = "https://example.com/A".into();
    downloader.title = "A".into();
    downloader.start_download(library.db.clone(), &ctx);
    assert_eq!(
        started.recv_timeout(Duration::from_secs(1)).unwrap(),
        ("https://example.com/A".into(), "A".into(), String::new())
    );
    assert!(downloader.url.is_empty() && downloader.title.is_empty());
    *downloader.progress.lock().unwrap() = 37.0;
    enqueue(&mut downloader, &library, &ctx, "B");
    enqueue(&mut downloader, &library, &ctx, "C");
    assert!(!downloader.start_download_url(
        "https://EXAMPLE.com/B#part".into(),
        "Duplicate".into(),
        "Other".into(),
        library.db.clone(),
        &ctx
    ));
    downloader.title = "Draft for the next song".into();
    assert_eq!(downloader.queue_len(), 3);
    assert_eq!(*downloader.progress.lock().unwrap(), 37.0);
    assert!(
        started.try_recv().is_err(),
        "Only one worker may run at once"
    );
    complete.send(Ok("A complete".into())).unwrap();
    assert_eq!(finish(&mut downloader).unwrap(), "A complete");
    assert_eq!(
        started.recv_timeout(Duration::from_secs(1)).unwrap(),
        ("https://example.com/B".into(), "B".into(), "Artist".into())
    );
    assert!(downloader.progress.lock().unwrap().is_nan());
    complete.send(Ok("B complete".into())).unwrap();
    finish(&mut downloader).unwrap();
    assert_eq!(started.recv_timeout(Duration::from_secs(1)).unwrap().1, "C");
    complete.send(Ok("C complete".into())).unwrap();
    finish(&mut downloader).unwrap();
    assert_eq!(downloader.queue_len(), 0);
    assert!(!downloader.busy);
    assert!(downloader
        .finished
        .iter()
        .all(|entry| matches!(entry.outcome, DownloadOutcome::Completed)));
}

#[test]
fn removal_failure_cancellation_and_retry_advance_without_blocking_the_queue() {
    let library = TestLibrary::new("download-recovery");
    let ctx = egui::Context::default();
    let (mut downloader, started, complete) = controlled();
    for name in ["A", "B", "C", "D"] {
        enqueue(&mut downloader, &library, &ctx, name);
    }
    assert_eq!(started.recv_timeout(Duration::from_secs(1)).unwrap().1, "A");
    let removed_id = downloader.pending[1].id;
    downloader.remove_pending(removed_id);
    complete.send(Err("Network unavailable".into())).unwrap();
    assert_eq!(finish(&mut downloader).unwrap_err(), "Network unavailable");
    assert_eq!(started.recv_timeout(Duration::from_secs(1)).unwrap().1, "B");
    let failed_id = downloader.finished.front().unwrap().job.id;
    downloader.retry(failed_id);
    assert_eq!(
        downloader
            .pending
            .iter()
            .map(|job| job.title.as_str())
            .collect::<Vec<_>>(),
        ["D", "A"]
    );
    downloader
        .download_cancel
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(finish(&mut downloader).is_err());
    assert_eq!(started.recv_timeout(Duration::from_secs(1)).unwrap().1, "D");
    assert!(downloader
        .finished
        .iter()
        .any(|entry| matches!(entry.outcome, DownloadOutcome::Cancelled)));
    assert!(
        !downloader
            .download_cancel
            .load(std::sync::atomic::Ordering::Relaxed),
        "Cancellation must not leak to the next job"
    );
    for name in ["D", "A"] {
        complete.send(Ok(format!("{name} complete"))).unwrap();
        finish(&mut downloader).unwrap();
        if name == "D" {
            assert_eq!(started.recv_timeout(Duration::from_secs(1)).unwrap().1, "A");
        }
    }
    assert_eq!(downloader.queue_len(), 0);
    assert!(
        started.try_recv().is_err(),
        "Removed song C must never start"
    );
}

#[test]
fn search_and_editing_remain_usable_while_a_worker_is_active() {
    let library = TestLibrary::new("download-independent-search");
    let ctx = egui::Context::default();
    let frost = crate::load_frost(&ctx);
    let (mut downloader, started, complete) = controlled();
    enqueue(&mut downloader, &library, &ctx, "A");
    started.recv_timeout(Duration::from_secs(1)).unwrap();
    let input = |events| egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(370.0, 700.0),
        )),
        events,
        ..Default::default()
    };
    run_ui(&ctx, input(vec![]), |ui| {
        downloader.show(ui, &ctx, &frost, &library.db);
    });
    ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("download-url-input")));
    run_ui(
        &ctx,
        input(vec![egui::Event::Text("https://example.com/next".into())]),
        |ui| {
            downloader.show(ui, &ctx, &frost, &library.db);
        },
    );
    assert_eq!(downloader.url, "https://example.com/next");
    ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("download-search-query")));
    run_ui(
        &ctx,
        input(vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]),
        |ui| {
            downloader.show(ui, &ctx, &frost, &library.db);
        },
    );
    assert!(
        downloader.search_error.is_some(),
        "Enter must still submit empty-query validation"
    );
    downloader.searching = true;
    downloader
        .search_tx
        .send(Ok(vec![SearchResult {
            title: "New result".into(),
            artist: "Artist".into(),
            duration: "3:00".into(),
            page_url: "https://example.com/new".into(),
            download_url: Some("https://example.com/new".into()),
        }]))
        .unwrap();
    assert!(downloader.poll().is_none());
    assert_eq!(downloader.results[0].title, "New result");
    assert!(downloader.busy && !downloader.searching);
    complete.send(Ok("Done".into())).unwrap();
    finish(&mut downloader).unwrap();
}

#[test]
fn queue_actions_fit_the_narrow_sidebar_and_follow_each_theme() {
    let library = TestLibrary::new("download-queue-themes");
    for appearance in [
        crate::Appearance::Aqua,
        crate::Appearance::Beer,
        crate::Appearance::Disco,
    ] {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::theme::install(&ctx, crate::GlassPalette::for_appearance(appearance));
        let frost = crate::load_frost(&ctx);
        let (mut downloader, started, complete) = controlled();
        enqueue(&mut downloader, &library, &ctx, "A");
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        enqueue(
            &mut downloader,
            &library,
            &ctx,
            "This is a very long pending song title that must remain inside the card",
        );
        let output = run_ui(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(224.0, 550.0),
                )),
                ..Default::default()
            },
            |ui| {
                downloader.show_queue(ui, &ctx, &frost, &library.db);
            },
        );
        let tree = output.platform_output.accesskit_update.as_ref().unwrap();
        for name in ["Anulează", "Elimină din coada de descărcare"] {
            let node = tree
                .nodes
                .iter()
                .find(|(_, node)| node.label() == Some(name))
                .unwrap();
            let bounds = node.1.bounds().unwrap();
            assert!(
                bounds.x0 >= 0.0 && bounds.x1 <= 224.0,
                "{name} clipped in {appearance:?}"
            );
        }
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == crate::theme::tint(&ctx, GLASS))), "Queue must use the active theme's glass");
        complete.send(Ok("Done".into())).unwrap();
        finish(&mut downloader).unwrap();
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        complete.send(Ok("Done".into())).unwrap();
        finish(&mut downloader).unwrap();
    }
}

#[test]
fn queue_shows_a_percentage_only_after_real_progress_is_available() {
    let library = TestLibrary::new("download-progress-ui");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let frost = crate::load_frost(&ctx);
    let (mut downloader, started, complete) = controlled();
    enqueue(&mut downloader, &library, &ctx, "A");
    started.recv_timeout(Duration::from_secs(1)).unwrap();
    for value in [f32::NAN, 37.0, f32::NAN] {
        *downloader.progress.lock().unwrap() = value;
        let output = run_ui(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(224.0, 550.0),
                )),
                ..Default::default()
            },
            |ui| downloader.show_queue(ui, &ctx, &frost, &library.db),
        );
        let nodes = &output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes;
        let texts = nodes
            .iter()
            .filter_map(|(_, node)| node.label().or_else(|| node.value()))
            .collect::<Vec<_>>();
        let percentages = nodes
            .iter()
            .filter(|(_, node)| node.role() == egui::accesskit::Role::ProgressIndicator)
            .filter_map(|(_, node)| node.numeric_value())
            .collect::<Vec<_>>();
        assert!(
            !texts.contains(&"0%"),
            "Unknown progress must not display a fake zero"
        );
        if value.is_nan() {
            assert!(texts.contains(&"Descărcare în curs..."));
            assert!(percentages.is_empty());
        } else {
            assert!(texts.contains(&"37%"));
            assert_eq!(percentages, [37.0]);
        }
    }
    complete.send(Ok("Done".into())).unwrap();
    finish(&mut downloader).unwrap();
}
