use crate::test_support::{wait_for_scans, TestLibrary};
use crate::*;

#[path = "home_scroll_tests.rs"]
mod home_scroll;

#[test]
fn outdated_library_results_do_not_overwrite_the_new_query() {
    let library = TestLibrary::new("refresh-order");
    library.mp3("Artist", "Alpha");
    library.mp3("Artist", "Beta");
    let ctx = egui::Context::default();
    let mut app =
        AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
    wait_for_scans(&mut app, &ctx);
    let old_version = app.refresh_version;
    app.search = "Beta".into();
    app.request_refresh(Duration::from_millis(50));
    let old = LibraryData {
        page_number: 99,
        page: Page {
            total: 999,
            tracks: vec![],
        },
        albums: vec![],
        playlists: vec![],
        roots: vec![],
    };
    app.tx
        .send(UiEvent::LibraryLoaded {
            version: old_version,
            result: Ok(old),
        })
        .unwrap();
    app.poll_events(&ctx);
    assert_ne!(app.page.total, 999);
    wait_for_scans(&mut app, &ctx);
    assert_eq!(app.page.total, 1);
    assert_eq!(app.page.tracks[0].title, "Beta");
}

#[test]
fn migration_backfills_unicode_keys_without_losing_user_data() {
    let library = TestLibrary::new("old-schema");
    library.mp3("Artist", "Șarpe");
    library.db.scan_downloaded_music(&library.music).unwrap();
    let track = library
        .db
        .page(0, 20, "", false, None)
        .unwrap()
        .tracks
        .remove(0);
    library.db.favorites(track.id).unwrap();
    library.db.note_play(track.id);
    {
        let connection = library.db.open().unwrap();
        connection.execute_batch("DROP TRIGGER tracks_key_insert;DROP TRIGGER tracks_key_update;DROP TRIGGER artists_key_insert;DROP TRIGGER artists_key_update;DROP TRIGGER albums_key_insert;DROP TRIGGER albums_key_update;
            DROP INDEX tracks_sort_title_ix;DROP INDEX tracks_sort_artist_title_ix;DROP INDEX tracks_sort_duration_ix;DROP INDEX artists_name_key_ix;DROP INDEX albums_title_key_ix;
            ALTER TABLE tracks DROP COLUMN title_key;ALTER TABLE artists DROP COLUMN name_key;ALTER TABLE albums DROP COLUMN title_key;").unwrap();
    }
    for _ in 0..2 {
        let reopened = Database::new(library.directory.join("library.sqlite3")).unwrap();
        let found = reopened
            .page(0, 20, "șarpe", true, None)
            .unwrap()
            .tracks
            .remove(0);
        assert_eq!(found.id, track.id);
        assert!(found.is_favorite);
        assert_eq!(found.play_count, 1);
        assert_eq!(found.file_path, track.file_path);
    }
    let backups: Vec<_> = std::fs::read_dir(&library.directory)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .contains("before-search-migration")
        })
        .collect();
    assert_eq!(backups.len(), 1);
    let backup = rusqlite::Connection::open(backups[0].path()).unwrap();
    assert_eq!(
        backup
            .query_row(
                "SELECT is_favorite,play_count FROM tracks WHERE id=?1",
                [track.id],
                |row| Ok((row.get::<_, bool>(0)?, row.get::<_, i64>(1)?))
            )
            .unwrap(),
        (true, 1)
    );
}

#[test]
fn invalid_changed_track_is_hidden_and_repaired_with_same_favorite() {
    let library = TestLibrary::new("invalid-repair");
    let path = library.mp3("Artist", "Song");
    library.db.scan(library.music.to_str().unwrap()).unwrap();
    let track = library
        .db
        .page(0, 20, "", false, None)
        .unwrap()
        .tracks
        .remove(0);
    library.db.favorites(track.id).unwrap();
    let original = std::fs::read(&path).unwrap();
    std::fs::write(&path, b"invalid audio").unwrap();
    let failed = library.db.scan(library.music.to_str().unwrap()).unwrap();
    assert_eq!(failed.errors, 1);
    assert!(library.db.get(track.id).unwrap().unwrap().is_missing);
    std::fs::write(&path, original).unwrap();
    library.db.scan(library.music.to_str().unwrap()).unwrap();
    let restored = library.db.get(track.id).unwrap().unwrap();
    assert!(!restored.is_missing);
    assert!(restored.is_favorite);
}

#[test]
fn qa_lrc_extreme_timestamp_is_rejected_without_panic() {
    let result = std::panic::catch_unwind(|| parse_lyrics("[18446744073709551615:59.99]hostile"));
    assert!(
        result.is_ok(),
        "Overflow while parsing a local LRC on the GUI thread"
    );
    assert!(
        result.unwrap().0.is_empty(),
        "An overflowing timestamp must be rejected"
    );
}

#[test]
fn qa_lrc_normal_negative_and_nonfinite_timestamps() {
    let (lines, _) = parse_lyrics("[00:00]start\n[01:59.99]end\n[-1:12]invalid\n[00:NaN]invalid\n[00:60]invalid\n[00:inf]invalid");
    assert_eq!(
        lines.iter().map(|l| l.time_ms).collect::<Vec<_>>(),
        [0, 119990]
    );
}

#[test]
fn qa_favorites_last_page_is_clamped_after_removing_its_only_track() {
    let library = TestLibrary::new("qa-pagination");
    for i in 0..21 {
        library.mp3("Artist", &format!("Song{i:02}"));
    }
    library.db.scan_downloaded_music(&library.music).unwrap();
    for id in library.db.order(None, false).unwrap() {
        library.db.favorites(id).unwrap();
    }
    let ctx = egui::Context::default();
    let mut app =
        AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
    wait_for_scans(&mut app, &ctx);
    app.navigate(Screen::Favorites);
    wait_for_scans(&mut app, &ctx);
    app.page_number = 1;
    app.refresh();
    wait_for_scans(&mut app, &ctx);
    library.db.favorites(app.page.tracks[0].id).unwrap();
    app.refresh();
    wait_for_scans(&mut app, &ctx);
    assert_eq!(
        (app.page_number, app.page.total, app.page.tracks.len()),
        (0, 20, 20),
        "Actual view becomes Page 2 / 1 with an empty table"
    );
}

#[test]
fn qa_unicode_search_is_case_insensitive_for_romanian() {
    let library = TestLibrary::new("qa-unicode");
    library.mp3("Artist", "Șarpe");
    library.db.scan_downloaded_music(&library.music).unwrap();
    assert_eq!(library.db.page(0, 20, "Ș", false, None).unwrap().total, 1);
    assert_eq!(
        library.db.page(0, 20, "ș", false, None).unwrap().total,
        1,
        "SQLite LIKE does not apply AURA_TEXT"
    );
}

#[test]
fn qa_search_treats_percent_and_underscore_as_literal_characters() {
    let library = TestLibrary::new("qa-wildcards");
    library.mp3("Artist", "Song");
    library.db.scan_downloaded_music(&library.music).unwrap();
    assert_eq!(library.db.page(0, 20, "%", false, None).unwrap().total, 0);
    assert_eq!(library.db.page(0, 20, "_", false, None).unwrap().total, 0);
}

#[test]
fn qa_sql_payload_is_bound_and_database_remains_intact() {
    let library = TestLibrary::new("qa-sql");
    library.mp3("Artist", "Song");
    library.db.scan_downloaded_music(&library.music).unwrap();
    assert_eq!(
        library
            .db
            .page(0, 20, "' OR 1=1; DROP TABLE tracks; --", false, None)
            .unwrap()
            .total,
        0
    );
    assert_eq!(library.db.page(0, 20, "", false, None).unwrap().total, 1);
}

#[test]
fn qa_page_api_clamps_invalid_and_extreme_arguments() {
    let library = TestLibrary::new("qa-page-boundary");
    library.mp3("Artist", "Song");
    library.db.scan_downloaded_music(&library.music).unwrap();
    assert_eq!(
        library
            .db
            .page(-1, 0, "", false, None)
            .unwrap()
            .tracks
            .len(),
        1
    );
    assert!(library
        .db
        .page(i64::MAX, i64::MAX, "", false, None)
        .unwrap()
        .tracks
        .is_empty());
    assert_eq!(
        library.db.page(0, 20, "", false, Some(-1)).unwrap().total,
        0
    );
    assert!(library.db.get(-1).unwrap().is_none());
    assert!(library.db.favorites(-1).is_err());
}

#[test]
fn qa_generic_import_rejects_invalid_audio_and_reports_errors() {
    let library = TestLibrary::new("qa-corrupt-import");
    std::fs::write(library.music.join("broken.mp3"), b"THIS IS NOT AUDIO").unwrap();
    let stats = library.db.scan(library.music.to_str().unwrap()).unwrap();
    assert_eq!(
        (
            stats.errors,
            library.db.page(0, 20, "", false, None).unwrap().total
        ),
        (1, 0),
        "Generic import reports success and publishes an unplayable track"
    );
}

#[test]
fn qa_missing_entire_music_root_marks_old_tracks_unavailable() {
    let library = TestLibrary::new("qa-missing-root");
    library.mp3("Artist", "Song");
    library.db.scan_downloaded_music(&library.music).unwrap();
    let moved = library.directory.join("Music-moved");
    std::fs::rename(&library.music, &moved).unwrap();
    let stats = library.db.scan_downloaded_music(&library.music).unwrap();
    assert_eq!(
        (
            stats.missing,
            library.db.page(0, 20, "", false, None).unwrap().total
        ),
        (1, 0)
    );
}

#[test]
fn qa_album_collection_can_exceed_240_albums() {
    let library = TestLibrary::new("qa-album-limit");
    let mut conn = library.db.open().unwrap();
    let tx = conn.transaction().unwrap();
    tx.execute("INSERT INTO artists(id,name) VALUES(1,'Artist')", [])
        .unwrap();
    for i in 1..=241 {
        tx.execute(
            "INSERT INTO albums(id,title,artist_id) VALUES(?1,?2,1)",
            rusqlite::params![i, format!("Album{i:03}")],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO tracks(file_path,title,album_id,artist_id) VALUES(?1,'Song',?2,1)",
            rusqlite::params![format!("C:/qa/{i}.mp3"), i],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    assert_eq!(
        library.db.albums("").unwrap().len(),
        241,
        "Unpaginated LIMIT 240 hides albums and caps dashboard statistics"
    );
}

#[test]
fn qa_refresh_is_responsive_while_background_writer_holds_database() {
    let library = TestLibrary::new("qa-db-lock");
    let ctx = egui::Context::default();
    let mut app =
        AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
    wait_for_scans(&mut app, &ctx);
    let conn = library.db.open().unwrap();
    conn.execute_batch(
        "PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE; BEGIN EXCLUSIVE;",
    )
    .unwrap();
    let unlock = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1200));
        conn.execute_batch("COMMIT").unwrap();
    });
    let start = Instant::now();
    app.refresh();
    let elapsed = start.elapsed();
    unlock.join().unwrap();
    wait_for_scans(&mut app, &ctx);
    println!("QA refresh with external exclusive database lock: {elapsed:?}");
    assert!(
        elapsed < Duration::from_millis(200),
        "GUI database access blocks the UI"
    );
}

#[test]
fn qa_corrupt_settings_are_reported_and_valid_saves_are_atomic() {
    let library = TestLibrary::new("qa-settings");
    let path = library.directory.join("settings.json");
    std::fs::write(&path, b"{truncated").unwrap();
    assert!(AppSettings::load(&path).is_err());
    let settings = AppSettings::default();
    settings.save(&path).unwrap();
    assert_eq!(AppSettings::load(&path).unwrap(), settings);
}

#[test]
fn qa_profile_large_library() {
    let library = TestLibrary::new("qa-performance");
    let mut conn = library.db.open().unwrap();
    let tx = conn.transaction().unwrap();
    tx.execute("INSERT INTO artists(id,name) VALUES(1,'Artist')", [])
        .unwrap();
    tx.execute(
        "INSERT INTO albums(id,title,artist_id) VALUES(1,'Album',1)",
        [],
    )
    .unwrap();
    {
        let mut insert = tx.prepare("INSERT INTO tracks(file_path,title,album_id,artist_id,duration_ms) VALUES(?1,?2,1,1,?3)").unwrap();
        for i in 0..50_000 {
            insert
                .execute(rusqlite::params![
                    format!("C:/qa/{i}.mp3"),
                    format!("Piesă {:05} ȘȚĂ", 50_000 - i),
                    i * 1000
                ])
                .unwrap();
        }
    }
    tx.commit().unwrap();
    for (name, sort) in [
        ("library", TrackSort::default()),
        (
            "title",
            TrackSort {
                column: SortColumn::Title,
                descending: false,
            },
        ),
        (
            "duration",
            TrackSort {
                column: SortColumn::Duration,
                descending: true,
            },
        ),
    ] {
        let start = Instant::now();
        let page = library
            .db
            .page_sorted(0, 20, "", false, None, sort)
            .unwrap();
        println!(
            "QA 50000 tracks page sort={name}: {:?}; total={}",
            start.elapsed(),
            page.total
        );
    }
    let start = Instant::now();
    let order = library
        .db
        .order_sorted(
            None,
            false,
            "",
            TrackSort {
                column: SortColumn::Title,
                descending: false,
            },
        )
        .unwrap();
    println!(
        "QA 50000 tracks build full title-sorted playback queue: {:?}; count={}",
        start.elapsed(),
        order.len()
    );
    assert_eq!(order.len(), 50_000);
}

#[test]
fn qa_all_importable_audio_formats_are_actually_decodable() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio");
    let library = TestLibrary::new("qa-format-matrix");
    let mut failures = vec![];
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let metadata = lofty::read_from_path(&path)
            .map(|_| ())
            .map_err(|e| e.to_string());
        if path
            .extension()
            .is_some_and(|extension| extension == "opus")
        {
            assert!(crate::database::validate_audio(&path)
                .unwrap_err()
                .contains("OPUS"));
            continue;
        }
        let decode = rodio::Decoder::try_from(std::fs::File::open(&path).unwrap());
        match decode {
            Ok(mut decoder) => {
                let count = decoder.by_ref().count();
                println!("QA FORMAT {name}: metadata={metadata:?}; decoded_samples={count}");
                if count == 0 {
                    failures.push(format!("{name}: no decoded samples"));
                }
            }
            Err(error) => {
                println!("QA FORMAT {name}: metadata={metadata:?}; decoder_error={error}");
                failures.push(format!("{name}: {error}"));
            }
        }
    }
    let stats = library
        .db
        .scan(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/audio")
                .to_str()
                .unwrap(),
        )
        .unwrap();
    println!(
        "QA FORMAT DB: discovered={} updated={} errors={}",
        stats.discovered, stats.updated, stats.errors
    );
    assert_eq!((stats.updated, stats.errors), (8, 1));
    assert!(stats.issues.iter().any(|issue| issue.contains("OPUS")));
    assert!(
        failures.is_empty(),
        "Supported extensions cannot play: {failures:?}"
    );
}
