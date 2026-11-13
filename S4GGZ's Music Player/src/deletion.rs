//! Confirmation policy and UI for permanent deletion of one selected audio file.
//! Only completed deletions enter the rolling, in-memory 60 second history.
use crate::{database::Track, AuraApp};
use eframe::egui;
use std::{
    collections::VecDeque,
    sync::mpsc,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct DeleteState {
    pub pending: Option<Track>,
    in_flight: Option<(i64, mpsc::Receiver<Result<(), String>>)>,
    recent: VecDeque<Instant>,
    close_after: bool,
}

impl DeleteState {
    pub fn confirmation_required(&mut self, now: Instant) -> bool {
        while self
            .recent
            .front()
            .is_some_and(|time| now.saturating_duration_since(*time) >= Duration::from_secs(60))
        {
            self.recent.pop_front();
        }
        self.recent.len() <= 3
    }
    fn record_success(&mut self, now: Instant) {
        self.confirmation_required(now);
        self.recent.push_back(now);
    }
    pub fn busy(&self) -> bool {
        self.in_flight.is_some()
    }
}

impl AuraApp {
    pub(super) fn defer_close_for_deletion(&mut self, ctx: &egui::Context) {
        if self.deletions.busy() && ctx.input(|input| input.viewport().close_requested()) {
            self.deletions.close_after = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
    }
    pub(super) fn request_track_deletion(&mut self, track: Track) {
        if self.deletions.busy() || self.deletions.pending.is_some() {
            return;
        }
        if self.deletions.confirmation_required(Instant::now()) {
            self.deletions.pending = Some(track);
        } else {
            self.start_track_deletion(track);
        }
    }
    fn start_track_deletion(&mut self, track: Track) {
        match self.player.delete_track(track.id, track.file_path) {
            Ok(receiver) => self.deletions.in_flight = Some((track.id, receiver)),
            Err(error) => self.toast(error),
        }
    }
    pub(super) fn poll_track_deletion(&mut self) {
        let Some((_, receiver)) = &self.deletions.in_flight else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Motorul audio s-a închis înainte de finalizarea ștergerii".into())
            }
        };
        let (id, _) = self.deletions.in_flight.take().unwrap();
        match result {
            Ok(()) => {
                self.deletions.record_success(Instant::now());
                self.page.tracks.retain(|track| track.id != id);
                self.queue_tracks.retain(|track| track.id != id);
                self.covers.remove(&id);
                self.pending_covers.remove(&id);
                self.refresh();
                self.sync_song();
                self.toast("Fișierul audio a fost șters definitiv");
            }
            Err(error) => self.toast(error),
        }
        if self.deletions.close_after {
            self.deletions.close_after = false;
            self.context.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
    pub(super) fn deletion_popup(&mut self, ctx: &egui::Context) {
        let Some(track) = self.deletions.pending.as_ref() else {
            return;
        };
        let mut confirm = false;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new("delete-audio-confirmation")).show(ctx, |ui| {
            ui.set_width(420.);
            ui.heading("Ștergi definitiv melodia?");
            ui.add_space(8.);
            ui.strong(&track.title);
            ui.label(&track.artist);
            ui.add_space(6.);
            ui.label(
                "Fișierul audio va fi șters de pe disc, fără a fi mutat în Coșul de reciclare.",
            );
            ui.label(egui::RichText::new(&track.file_path).small().weak());
            ui.add_space(12.);
            ui.horizontal(|ui| {
                cancel = ui.button("Anulează").clicked();
                confirm = ui.button("Șterge definitiv").clicked();
            });
        });
        if confirm {
            if let Some(track) = self.deletions.pending.take() {
                self.start_track_deletion(track);
            }
        } else if cancel || modal.should_close() {
            self.deletions.pending = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fifth_deletion_skips_confirmation_only_while_four_successes_are_recent() {
        let mut state = DeleteState::default();
        let now = Instant::now();
        for second in [0, 10, 20, 30] {
            assert!(state.confirmation_required(now + Duration::from_secs(second)));
            state.record_success(now + Duration::from_secs(second));
        }
        assert!(!state.confirmation_required(now + Duration::from_secs(59)));
        assert!(state.confirmation_required(now + Duration::from_secs(60)));
        assert!(state.confirmation_required(now + Duration::from_secs(91)));
        assert!(state.recent.is_empty());
    }

    #[test]
    fn permanent_deletion_removes_only_the_selected_file_catalog_and_playlist_entry() {
        let library = crate::test_support::TestLibrary::new("delete-file");
        let selected = library.mp3("Artist", "Selected");
        let neighbor = library.mp3("Artist", "Keep");
        let cover = library.music.join("cover.png");
        std::fs::write(&cover, b"keep this cover").unwrap();
        let neighbor_bytes = std::fs::read(&neighbor).unwrap();
        library.db.scan_downloaded_music(&library.music).unwrap();
        let track = library
            .db
            .page(0, 20, "Selected", false, None)
            .unwrap()
            .tracks
            .remove(0);
        let connection = library.db.open().unwrap();
        connection
            .execute("INSERT INTO playlists(title) VALUES('Test')", [])
            .unwrap();
        let playlist = connection.last_insert_rowid();
        connection
            .execute(
                "INSERT INTO playlist_tracks(playlist_id,track_id,position) VALUES(?1,?2,0)",
                rusqlite::params![playlist, track.id],
            )
            .unwrap();
        drop(connection);
        library
            .db
            .delete_audio_file(track.id, &track.file_path)
            .unwrap();
        assert!(!selected.exists());
        assert!(library.db.get(track.id).unwrap().is_none());
        assert_eq!(std::fs::read(neighbor).unwrap(), neighbor_bytes);
        assert_eq!(std::fs::read(cover).unwrap(), b"keep this cover");
        let count: i64 = library
            .db
            .open()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM playlist_tracks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
        library.db.scan_downloaded_music(&library.music).unwrap();
        assert_eq!(library.db.page(0, 20, "", false, None).unwrap().total, 1);
    }

    #[test]
    fn stale_selection_and_locked_file_do_not_delete_the_catalog_or_other_files() {
        use std::os::windows::fs::OpenOptionsExt;
        let library = crate::test_support::TestLibrary::new("delete-locked");
        let path = library.mp3("Artist", "Locked");
        library.db.scan_downloaded_music(&library.music).unwrap();
        let track = library
            .db
            .page(0, 20, "", false, None)
            .unwrap()
            .tracks
            .remove(0);
        assert!(library.db.delete_audio_file(track.id, "wrong.mp3").is_err());
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        assert!(library
            .db
            .delete_audio_file(track.id, &track.file_path)
            .is_err());
        assert!(library.db.get(track.id).unwrap().is_some());
        drop(held);
        assert!(path.exists());
        library
            .db
            .delete_audio_file(track.id, &track.file_path)
            .unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn cancelling_the_popup_keeps_the_file_and_does_not_enable_fast_deletion() {
        let library = crate::test_support::TestLibrary::new("delete-cancel");
        let path = library.mp3("Artist", "Keep");
        library.db.scan_downloaded_music(&library.music).unwrap();
        let track = library
            .db
            .page(0, 20, "", false, None)
            .unwrap()
            .tracks
            .remove(0);
        let ctx = egui::Context::default();
        let mut app =
            AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
        crate::test_support::wait_for_scans(&mut app, &ctx);
        app.request_track_deletion(track);
        assert!(app.deletions.pending.is_some());
        let _ = crate::test_support::run_ui(&ctx, egui::RawInput::default(), |_| {
            app.deletion_popup(&ctx)
        });
        let _ = crate::test_support::run_ui(
            &ctx,
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
                ..Default::default()
            },
            |_| app.deletion_popup(&ctx),
        );
        assert!(app.deletions.pending.is_none());
        assert!(!app.deletions.busy());
        assert!(app.deletions.recent.is_empty());
        assert!(path.exists());
    }

    #[test]
    fn only_worker_confirmed_successes_enable_automatic_deletion() {
        let library = crate::test_support::TestLibrary::new("delete-history");
        for index in 0..6 {
            library.mp3("Artist", &format!("Song {index}"));
        }
        library.db.scan_downloaded_music(&library.music).unwrap();
        let tracks = library.db.page(0, 20, "", false, None).unwrap().tracks;
        let ctx = egui::Context::default();
        let mut app =
            AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
        crate::test_support::wait_for_scans(&mut app, &ctx);
        let finish = |app: &mut AuraApp| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while app.deletions.busy() {
                assert!(Instant::now() < deadline);
                app.poll_track_deletion();
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        for track in &tracks[..4] {
            app.request_track_deletion(track.clone());
            let confirmed = app.deletions.pending.take().unwrap();
            app.start_track_deletion(confirmed);
            finish(&mut app);
        }
        assert_eq!(app.deletions.recent.len(), 4);
        app.request_track_deletion(tracks[4].clone());
        assert!(app.deletions.pending.is_none());
        assert!(app.deletions.busy());
        finish(&mut app);
        assert!(!std::path::Path::new(&tracks[4].file_path).exists());
        let (sender, receiver) = mpsc::channel();
        sender.send(Err("Fixture: fișier blocat".into())).unwrap();
        app.deletions.in_flight = Some((tracks[5].id, receiver));
        app.poll_track_deletion();
        assert_eq!(app.deletions.recent.len(), 5);
        assert!(std::path::Path::new(&tracks[5].file_path).exists());
        crate::test_support::wait_for_scans(&mut app, &ctx);
    }
}
