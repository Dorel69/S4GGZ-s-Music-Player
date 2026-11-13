use crate::database::Database;
use crate::music_search::{self, Platform, SearchResult};
use crate::process_runner::{self, Cancellation};
use eframe::egui::{self, Color32, RichText, TextureHandle};
use std::{
    collections::VecDeque,
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const GLASS: Color32 = Color32::from_rgba_premultiplied(12, 28, 43, 170);
const TEXT: Color32 = Color32::from_rgb(244, 242, 252);
const MUTED: Color32 = Color32::from_rgb(159, 162, 193);

#[cfg(test)]
#[path = "download_queue_tests.rs"]
mod queue_tests;
#[path = "download_queue_ui.rs"]
mod queue_ui;

type DownloadExecutor = dyn Fn(
        &str,
        &str,
        &str,
        &DownloadOptions,
        &Database,
        Arc<Mutex<f32>>,
        &Cancellation,
    ) -> Result<String, String>
    + Send
    + Sync;

#[derive(Clone)]
struct DownloadJob {
    id: u64,
    url: String,
    key: String,
    title: String,
    artist: String,
    options: DownloadOptions,
    db: Database,
    ctx: egui::Context,
}

#[derive(Clone, Debug, Default)]
struct DownloadOptions {
    playlist_album: Option<String>,
    playlist_id: Option<i64>,
    cookie_browser: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum PlaylistSaveMode {
    #[default]
    Album,
    Playlist,
}

impl DownloadJob {
    fn label(&self) -> &str {
        if self.title.is_empty() {
            &self.url
        } else {
            &self.title
        }
    }
}

enum DownloadOutcome {
    Completed,
    Failed(String),
    Cancelled,
}

struct FinishedDownload {
    job: DownloadJob,
    outcome: DownloadOutcome,
}

pub struct Downloader {
    url: String,
    title: String,
    platform: Platform,
    cookie_browser: Option<String>,
    query: String,
    results: Vec<SearchResult>,
    searching: bool,
    search_error: Option<String>,
    busy: bool,
    progress: Arc<Mutex<f32>>,
    status: Option<Result<String, String>>,
    tx: Sender<Result<String, String>>,
    rx: Receiver<Result<String, String>>,
    search_tx: Sender<Result<Vec<SearchResult>, String>>,
    search_rx: Receiver<Result<Vec<SearchResult>, String>>,
    playlist_tx: Sender<Result<music_search::PlaylistResult, String>>,
    playlist_rx: Receiver<Result<music_search::PlaylistResult, String>>,
    playlist_context: Option<(Database, egui::Context, PlaylistSaveMode)>,
    playlist_save_mode: PlaylistSaveMode,
    download_cancel: Cancellation,
    search_cancel: Cancellation,
    active: Option<DownloadJob>,
    pending: VecDeque<DownloadJob>,
    finished: VecDeque<FinishedDownload>,
    next_job_id: u64,
    executor: Arc<DownloadExecutor>,
}

impl Downloader {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let (search_tx, search_rx) = mpsc::channel();
        let (playlist_tx, playlist_rx) = mpsc::channel();
        Self {
            url: String::new(),
            title: String::new(),
            platform: Platform::YouTube,
            cookie_browser: None,
            query: String::new(),
            results: Vec::new(),
            searching: false,
            search_error: None,
            busy: false,
            progress: Arc::new(Mutex::new(f32::NAN)),
            status: None,
            tx,
            rx,
            search_tx,
            search_rx,
            playlist_tx,
            playlist_rx,
            playlist_context: None,
            playlist_save_mode: PlaylistSaveMode::default(),
            download_cancel: process_runner::cancellation(),
            search_cancel: process_runner::cancellation(),
            active: None,
            pending: VecDeque::new(),
            finished: VecDeque::new(),
            next_job_id: 0,
            executor: Arc::new(download_and_import),
        }
    }

    pub fn set_cookie_browser(&mut self, browser: Option<String>) {
        self.cookie_browser = browser.filter(|name| {
            matches!(
                name.as_str(),
                "chrome" | "edge" | "firefox" | "brave" | "opera" | "vivaldi"
            )
        });
    }

    pub fn cookie_browser(&self) -> Option<&str> {
        self.cookie_browser.as_deref()
    }

    /// Process worker results even when the download screen is closed.
    pub fn poll(&mut self) -> Option<Result<String, String>> {
        if let Ok(result) = self.search_rx.try_recv() {
            self.searching = false;
            match result {
                Ok(results) => {
                    self.results = results;
                    self.search_error = None;
                }
                Err(error) => {
                    self.results.clear();
                    self.search_error = Some(error);
                }
            }
        }
        if let Ok(result) = self.playlist_rx.try_recv() {
            self.searching = false;
            match result {
                Ok(playlist) => {
                    if let Some((db, ctx, mode)) = self.playlist_context.take() {
                        let title = playlist.title;
                        let playlist_id = if mode == PlaylistSaveMode::Playlist {
                            match db.create_or_get_playlist(&title) {
                                Ok(id) => Some(id),
                                Err(error) => {
                                    self.search_error = Some(error);
                                    None
                                }
                            }
                        } else {
                            None
                        };
                        if mode == PlaylistSaveMode::Playlist && playlist_id.is_none() {
                            return None;
                        }
                        let mut queued = 0;
                        for track in playlist.tracks {
                            if let Some(url) = track.download_url {
                                queued += usize::from(
                                    self.start_download_url_with(
                                        url,
                                        track.title,
                                        track.artist,
                                        DownloadOptions {
                                            playlist_album: (mode == PlaylistSaveMode::Album)
                                                .then(|| title.clone()),
                                            playlist_id,
                                            cookie_browser: self.cookie_browser.clone(),
                                        },
                                        db.clone(),
                                        &ctx,
                                    ),
                                );
                            }
                        }
                        self.status = Some(Ok(match mode {
                            PlaylistSaveMode::Album => format!(
                                "{queued} melodii din playlist au fost adăugate în coadă ca albumul „{title}”."
                            ),
                            PlaylistSaveMode::Playlist => format!(
                                "{queued} melodii au fost adăugate în coadă pentru playlistul „{title}”; piesele vor apărea și în albumul „Descarcate”."
                            ),
                        }));
                    }
                }
                Err(error) => {
                    self.playlist_context = None;
                    self.search_error = Some(error);
                }
            }
        }
        if let Ok(result) = self.rx.try_recv() {
            self.busy = false;
            self.status = Some(result.clone());
            if let Some(job) = self.active.take() {
                let outcome = match &result {
                    Ok(_) => DownloadOutcome::Completed,
                    Err(_)
                        if self
                            .download_cancel
                            .load(std::sync::atomic::Ordering::Relaxed) =>
                    {
                        DownloadOutcome::Cancelled
                    }
                    Err(error) => DownloadOutcome::Failed(error.clone()),
                };
                self.finished.push_front(FinishedDownload { job, outcome });
                self.finished.truncate(20);
            }
            self.start_next();
            return Some(result);
        }
        None
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        frost: &TextureHandle,
        db: &Database,
    ) -> egui::scroll_area::ScrollAreaOutput<()> {
        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
        egui::ScrollArea::vertical()
            .id_salt("download-page")
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .max_height(ui.available_height())
            .show(ui, |ui| {
        let panel = egui::Frame::new()
            .fill(crate::theme::tint(ui.ctx(), GLASS))
            .corner_radius(16)
            .inner_margin(egui::Margin::same(22))
            .stroke(egui::Stroke::new(
                1.0,
                Color32::from_rgba_unmultiplied(255, 255, 255, 35),
            ))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical(|ui| {
                    ui.label(RichText::new("Descarcă melodii").size(26.0).strong().color(TEXT));
                    ui.add_space(5.0);
                    ui.label(RichText::new("Descarcă fișiere audio oferite direct și autorizat de creator.").size(13.0).color(MUTED));
                    ui.add_space(22.0);

                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("COOKIE-URI YOUTUBE").size(11.0).strong().color(crate::theme::accent(ui.ctx())));
                        egui::ComboBox::from_id_salt("youtube-cookie-browser")
                            .selected_text(self.cookie_browser.as_deref().map(browser_label).unwrap_or("Dezactivat"))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.cookie_browser, None, "Dezactivat");
                                for (id, label) in [("chrome", "Chrome"), ("edge", "Microsoft Edge"), ("firefox", "Firefox"), ("brave", "Brave"), ("opera", "Opera"), ("vivaldi", "Vivaldi")] {
                                    ui.selectable_value(&mut self.cookie_browser, Some(id.to_owned()), label);
                                }
                            });
                        ui.label(RichText::new("Alege browserul în care ești autentificat pentru clipurile cu limită de vârstă.").size(11.0).color(MUTED));
                    });
                    ui.add_space(14.0);

                    ui.label(RichText::new("CAUTĂ PE PLATFORMĂ").size(11.0).strong().color(crate::theme::accent(ui.ctx())));
                    ui.add_space(6.0);
                    ui.add_enabled_ui(!self.searching, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            egui::ComboBox::from_id_salt("music-search-platform")
                                .selected_text(self.platform.label())
                                .width(125.0)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut self.platform, Platform::YouTube, "YouTube");
                                    ui.selectable_value(&mut self.platform, Platform::SoundCloud, "SoundCloud");
                                });
                            let enter_pressed = ui.input(|input| input.key_pressed(egui::Key::Enter));
                            let search_input = ui.add(
                                egui::TextEdit::singleline(&mut self.query)
                                    .char_limit(512)
                                    .id(egui::Id::new("download-search-query"))
                                    .desired_width((ui.available_width() - 90.0).max(80.0))
                                    .hint_text("Caută melodii sau artiști")
                                    .margin(egui::Margin::symmetric(10, 8)),
                            );
                            let submit = enter_pressed && (search_input.has_focus() || search_input.lost_focus());
                            if ui.button("Caută").clicked() || submit {
                                self.start_search();
                            }
                        });
                    });
                    if self.searching {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new(if self.playlist_context.is_some() { "Se citesc melodiile din playlist..." } else { "Se caută primele 10 rezultate..." }).color(MUTED));
                        });
                        if ui.button("Anulează căutarea").clicked(){self.search_cancel.store(true,std::sync::atomic::Ordering::Relaxed);}
                        ctx.request_repaint_after(Duration::from_millis(120));
                    }
                    if let Some(error) = &self.search_error {
                        ui.label(RichText::new(error).size(12.0).color(Color32::from_rgb(255, 150, 130)));
                    }
                    let mut selected_result = None;
                    if !self.results.is_empty() {
                        egui::ScrollArea::vertical()
                            .id_salt("music-search-results")
                            .max_height(240.0)
                            .show(ui, |ui| {
                                for result in &self.results {
                                    egui::Frame::new()
                                        .fill({ let palette = crate::theme::current(ui.ctx()); if palette == crate::theme::GlassPalette::default() { Color32::from_rgba_premultiplied(8, 24, 39, 110) } else { palette.card.gamma_multiply(0.65) } })
                                        .corner_radius(8)
                                        .inner_margin(egui::Margin::symmetric(10, 7))
                                        .show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            ui.add(egui::Label::new(RichText::new(&result.title).strong().color(TEXT)).truncate()).on_hover_text(&result.title);
                                            ui.add(egui::Label::new(RichText::new(format!("{}  ·  {}", result.artist, result.duration)).size(12.0).color(MUTED)).truncate());
                                            ui.horizontal_wrapped(|ui| {
                                                    let queued = result.download_url.as_deref().is_some_and(|url| self.contains_url(url));
                                                    let enabled = result.download_url.is_some() && !queued;
                                                    let label = if queued { "În coadă" } else if result.download_url.is_some() { "Descarcă" } else { "Indisponibil" };
                                                    ui.hyperlink_to("Deschide", &result.page_url);
                                                    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                                                        selected_result = Some(result.clone());
                                                    }
                                            });
                                        });
                                }
                            });
                    }
                    if let Some(result) = selected_result {
                        if let Some(url) = result.download_url {
                            self.start_download_url(url, result.title, result.artist, db.clone(), ctx);
                        }
                    }
                    ui.add_space(18.0);

                    ui.label(RichText::new("LINK DIRECT SAU PLATFORMĂ").size(11.0).strong().color(crate::theme::accent(ui.ctx())));
                    ui.add_space(6.0);
                    ui.scope(|ui| {
                        ui.add_sized(
                            [ui.available_width(), 42.0],
                            egui::TextEdit::singleline(&mut self.url)
                                .char_limit(8192)
                                .id(egui::Id::new("download-url-input"))
                                .hint_text("https://... (YouTube, SoundCloud sau fișier audio)")
                                .margin(egui::Margin::symmetric(12, 10)),
                        );
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Titlu").color(MUTED));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.title)
                                    .char_limit(512)
                                    .desired_width(ui.available_width())
                                    .hint_text("Numele melodiei")
                                    .margin(egui::Margin::symmetric(10, 8)),
                            );
                        });
                    });
                    ui.add_space(16.0);
                    let can_download = !self.url.trim().is_empty() && !self.contains_url(self.url.trim());
                    ui.horizontal(|ui| {
                        ui.set_min_height(36.0);
                        if ui.add_enabled(can_download, egui::Button::new(
                            RichText::new(if self.busy { "+  Adaugă la coadă" } else { "↓  Descarcă" }).strong().color(TEXT),
                        ).fill(crate::theme::accent(ui.ctx()).gamma_multiply(0.45)).corner_radius(8)).clicked() {
                            self.start_download(db.clone(), ctx);
                        }
                        let playlist_url = self.url.trim().to_owned();
                        if ui.add_enabled(!self.searching && is_http_url(&playlist_url), egui::Button::new("↓  Descarcă playlist")).clicked() {
                            self.start_playlist(playlist_url, db.clone(), ctx);
                        }
                        ui.label(RichText::new("Salvează ca").color(MUTED));
                        egui::ComboBox::from_id_salt("playlist-download-save-mode")
                            .selected_text(match self.playlist_save_mode {
                                PlaylistSaveMode::Album => "Album separat",
                                PlaylistSaveMode::Playlist => "Playlist",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.playlist_save_mode,
                                    PlaylistSaveMode::Album,
                                    "Album separat",
                                );
                                ui.selectable_value(
                                    &mut self.playlist_save_mode,
                                    PlaylistSaveMode::Playlist,
                                    "Playlist",
                                );
                            });
                    });
                    if self.busy {
                        ui.label(RichText::new("Poți căuta și adăuga alte melodii. Descărcările continuă în coada din dreapta.").size(12.0).color(MUTED));
                        ctx.request_repaint_after(Duration::from_millis(120));
                    }
                    if let Some(status) = &self.status {
                        ui.add_space(10.0);
                        match status {
                            Ok(message) => { ui.label(RichText::new(message).color(crate::theme::accent(ui.ctx()))); }
                            Err(message) => { ui.label(RichText::new(message).color(Color32::from_rgb(255, 120, 120))); }
                        }
                    }
                    ui.add_space(12.0);
                    ui.label(RichText::new("Descarcă doar conținut pe care ai dreptul să îl salvezi.").size(12.0).color(MUTED));
                });
            });
        crate::paint_frost_grain_in_rect(ui, frost, panel.response.rect.shrink(22.0));
        ui.painter().rect_stroke(
            panel.response.rect,
            16,
            egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 35)),
            egui::StrokeKind::Inside,
        );
        ui.painter().line_segment(
            [
                panel.response.rect.left_top() + egui::vec2(20.0, 1.0),
                panel.response.rect.right_top() - egui::vec2(20.0, -1.0),
            ],
            egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 35)),
        );
        })
    }

    fn start_download(&mut self, db: Database, ctx: &egui::Context) {
        let url = self.url.trim().to_owned();
        if !is_http_url(&url) {
            self.status = Some(Err("Introdu un URL valid HTTP sau HTTPS.".into()));
            return;
        }
        if self.start_download_url(url, self.title.trim().to_owned(), String::new(), db, ctx) {
            self.url.clear();
            self.title.clear();
        }
    }

    fn start_playlist(&mut self, url: String, db: Database, ctx: &egui::Context) {
        self.search_error = None;
        self.searching = true;
        self.playlist_context = Some((db, ctx.clone(), self.playlist_save_mode));
        self.search_cancel = process_runner::cancellation();
        let cancel = self.search_cancel.clone();
        let tx = self.playlist_tx.clone();
        let repaint = ctx.clone();
        let browser = self.cookie_browser.clone();
        thread::spawn(move || {
            let result = music_search::playlist(&url, &cancel, browser.as_deref());
            let _ = tx.send(result);
            repaint.request_repaint();
        });
    }

    fn start_download_url(
        &mut self,
        url: String,
        title: String,
        artist: String,
        db: Database,
        ctx: &egui::Context,
    ) -> bool {
        self.start_download_url_with(
            url,
            title,
            artist,
            DownloadOptions {
                playlist_album: None,
                playlist_id: None,
                cookie_browser: self.cookie_browser.clone(),
            },
            db,
            ctx,
        )
    }

    fn start_download_url_with(
        &mut self,
        url: String,
        title: String,
        artist: String,
        options: DownloadOptions,
        db: Database,
        ctx: &egui::Context,
    ) -> bool {
        self.start_download_configured(url, title, artist, options, db, ctx)
    }

    fn start_download_configured(
        &mut self,
        url: String,
        title: String,
        artist: String,
        options: DownloadOptions,
        db: Database,
        ctx: &egui::Context,
    ) -> bool {
        let url = url.trim().to_owned();
        if !is_http_url(&url) {
            self.status = Some(Err("Introdu un URL valid HTTP sau HTTPS.".into()));
            return false;
        }
        if self.contains_url(&url) {
            self.status = Some(Err("Melodia este deja în coada de descărcare.".into()));
            return false;
        }
        self.next_job_id = self.next_job_id.wrapping_add(1);
        self.pending.push_back(DownloadJob {
            id: self.next_job_id,
            key: url_key(&url),
            url,
            title: title.trim().to_owned(),
            artist: artist.trim().to_owned(),
            options: DownloadOptions {
                playlist_album: options
                    .playlist_album
                    .map(|album| album.trim().to_owned())
                    .filter(|album| !album.is_empty()),
                ..options
            },
            db,
            ctx: ctx.clone(),
        });
        self.status = None;
        self.start_next();
        true
    }

    pub fn queue_len(&self) -> usize {
        usize::from(self.active.is_some()) + self.pending.len()
    }

    fn contains_url(&self, url: &str) -> bool {
        let key = url_key(url);
        self.active.as_ref().is_some_and(|job| job.key == key)
            || self.pending.iter().any(|job| job.key == key)
    }

    fn start_next(&mut self) {
        if self.busy || self.active.is_some() {
            return;
        }
        let Some(job) = self.pending.pop_front() else {
            return;
        };
        self.download_cancel = process_runner::cancellation();
        let cancel = self.download_cancel.clone();
        let tx = self.tx.clone();
        // No percentage is known until the worker reports one for this job.
        self.progress = Arc::new(Mutex::new(f32::NAN));
        let progress = self.progress.clone();
        let repaint = job.ctx.clone();
        let executor = self.executor.clone();
        self.active = Some(job.clone());
        self.busy = true;
        thread::spawn(move || {
            let result = executor(
                &job.url,
                &job.title,
                &job.artist,
                &job.options,
                &job.db,
                progress,
                &cancel,
            );
            let _ = tx.send(result);
            repaint.request_repaint();
        });
    }

    fn remove_pending(&mut self, id: u64) {
        self.pending.retain(|job| job.id != id);
    }

    fn retry(&mut self, id: u64) {
        let Some(index) = self.finished.iter().position(|entry| entry.job.id == id) else {
            return;
        };
        if matches!(self.finished[index].outcome, DownloadOutcome::Completed) {
            return;
        }
        let job = self.finished[index].job.clone();
        if self.start_download_configured(
            job.url,
            job.title,
            job.artist,
            job.options,
            job.db,
            &job.ctx,
        ) {
            self.finished.remove(index);
        }
    }

    fn start_search(&mut self) {
        if self.searching {
            return;
        }
        let query = self.query.trim().to_owned();
        if query.is_empty() {
            self.search_error = Some("Introdu un termen de căutare.".into());
            return;
        }
        self.searching = true;
        self.results.clear();
        self.search_error = None;
        let platform = self.platform;
        let browser = self.cookie_browser.clone();
        let tx = self.search_tx.clone();
        self.search_cancel = process_runner::cancellation();
        let cancel = self.search_cancel.clone();
        thread::spawn(move || {
            let _ = tx.send(music_search::search(
                platform,
                &query,
                &cancel,
                browser.as_deref(),
            ));
        });
    }
}

fn url_key(url: &str) -> String {
    reqwest::Url::parse(url.trim()).map_or_else(
        |_| url.trim().to_owned(),
        |mut url| {
            url.set_fragment(None);
            url.to_string()
        },
    )
}

fn browser_label(browser: &str) -> &'static str {
    match browser {
        "chrome" => "Chrome",
        "edge" => "Microsoft Edge",
        "firefox" => "Firefox",
        "brave" => "Brave",
        "opera" => "Opera",
        "vivaldi" => "Vivaldi",
        _ => "Browser nesuportat",
    }
}

fn explain_youtube_auth_error(error: String, browser: Option<&str>) -> String {
    if error.contains("Sign in to confirm your age") {
        let setup = browser.map(browser_label)
            .map(|name| format!("Verifică autentificarea în {name} și închide complet browserul înainte de reîncercare."))
            .unwrap_or_else(|| "Selectează browserul în care ești autentificat din lista COOKIE-URI YOUTUBE.".into());
        format!("{error}\n\nYouTube cere o sesiune autentificată. {setup} Cookie-urile sunt citite la cerere și nu sunt salvate de Aura Player.")
    } else {
        error
    }
}

impl Drop for Downloader {
    fn drop(&mut self) {
        self.download_cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.search_cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

fn sanitize_filename(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '%' => '_',
            character if character.is_control() => '_',
            character => character,
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.');
    if cleaned.is_empty() {
        "melodie".into()
    } else {
        let cleaned: String = cleaned.chars().take(80).collect();
        let cleaned = cleaned.trim_end_matches([' ', '.']);
        if cleaned.is_empty() {
            return "melodie".into();
        }
        let device_name = cleaned
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        let reserved = matches!(device_name.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ["COM", "LPT"].iter().any(|prefix| {
                device_name.strip_prefix(prefix).is_some_and(|suffix| {
                    matches!(
                        suffix,
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                    )
                })
            });
        if reserved {
            format!("_{cleaned}")
        } else {
            cleaned.to_owned()
        }
    }
}

fn title_without_author(title: &str, artist: &str) -> String {
    let title = title.trim();
    let artist = artist.trim();
    if artist.is_empty() {
        return title.to_owned();
    }
    for separator in [" - ", " – ", " — "] {
        if let Some((left, right)) = title.split_once(separator) {
            if left.trim().eq_ignore_ascii_case(artist) && !right.trim().is_empty() {
                return right.trim().to_owned();
            }
            if right.trim().eq_ignore_ascii_case(artist) && !left.trim().is_empty() {
                return left.trim().to_owned();
            }
        }
    }
    title.to_owned()
}

fn track_names(
    preferred_title: &str,
    preferred_artist: &str,
    platform_title: &str,
    platform_artist: &str,
) -> (String, String) {
    let artist = if preferred_artist.trim().is_empty() {
        platform_artist.trim()
    } else {
        preferred_artist.trim()
    };
    let artist = if artist.is_empty() {
        "Unknown Artist"
    } else {
        artist
    };
    let title = if preferred_title.trim().is_empty() {
        title_without_author(platform_title, artist)
    } else {
        preferred_title.trim().to_owned()
    };
    (sanitize_filename(&title), sanitize_filename(artist))
}

fn unique_track_stem(directory: &Path, title: &str) -> String {
    if !directory.join(format!("{title}.mp3")).exists() {
        return title.to_owned();
    }
    (2..)
        .map(|index| format!("{title} ({index})"))
        .find(|stem| !directory.join(format!("{stem}.mp3")).exists())
        .unwrap_or_else(|| format!("{title}-{}", std::process::id()))
}

fn is_http_url(url: &str) -> bool {
    if url.len() > 8192 {
        return false;
    }
    if url
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        return false;
    }
    reqwest::Url::parse(url).ok().is_some_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some_and(|host| !host.is_empty())
    })
}

pub fn music_directory() -> Result<PathBuf, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("Nu am putut găsi folderul programului: {error}"))?;
    Ok(executable
        .parent()
        .ok_or("Executabilul nu are un folder părinte")?
        .join("Music"))
}

fn find_ffmpeg_directory(program_directory: &Path, search_path: Option<&OsStr>) -> Option<PathBuf> {
    let mut directories = vec![
        program_directory.join("ffmpeg").join("bin"),
        program_directory.join("bin"),
        program_directory.to_owned(),
    ];
    if let Some(search_path) = search_path {
        directories.extend(std::env::split_paths(search_path));
    }
    let suffix = std::env::consts::EXE_SUFFIX;
    directories.into_iter().find(|directory| {
        directory.join(format!("ffmpeg{suffix}")).is_file()
            && directory.join(format!("ffprobe{suffix}")).is_file()
    })
}

fn ffmpeg_directory() -> Result<PathBuf, String> {
    let music = music_directory()?;
    let program = music
        .parent()
        .ok_or("Folderul programului nu este disponibil")?;
    find_ffmpeg_directory(program, std::env::var_os("PATH").as_deref()).ok_or_else(|| {
        "Conversia MP3 necesită FFmpeg și FFprobe. Pune executabilele în ffmpeg/bin lângă program sau adaugă-le în PATH, apoi reîncearcă descărcarea.".into()
    })
}

/// Keep intermediate audio outside Music so automatic scans see completed files only.
struct DownloadWorkspace(PathBuf);

impl DownloadWorkspace {
    fn new(music_directory: &Path) -> Result<Self, String> {
        let parent = music_directory
            .parent()
            .ok_or("Folderul Music nu are un părinte")?
            .join(".aura-downloads");
        std::fs::create_dir_all(&parent).map_err(|error| error.to_string())?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let path = parent.join(format!("{}-{timestamp}", std::process::id()));
        std::fs::create_dir(&path).map_err(|error| error.to_string())?;
        Ok(Self(path))
    }
}

impl Drop for DownloadWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
        if let Some(parent) = self.0.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

fn download_and_import(
    url: &str,
    preferred_title: &str,
    preferred_artist: &str,
    options: &DownloadOptions,
    db: &Database,
    progress: Arc<Mutex<f32>>,
    cancel: &Cancellation,
) -> Result<String, String> {
    process_runner::check_cancel(cancel)?;
    let ffmpeg = ffmpeg_directory()?;
    let (platform_title, platform_artist) =
        if preferred_title.trim().is_empty() || preferred_artist.trim().is_empty() {
            crate::yt_dlp::metadata_with_cookies(url, cancel, options.cookie_browser.as_deref())
                .unwrap_or_else(|_| (preferred_title.to_owned(), preferred_artist.to_owned()))
        } else {
            (preferred_title.to_owned(), preferred_artist.to_owned())
        };
    process_runner::check_cancel(cancel)?;
    let (title, artist) = track_names(
        preferred_title,
        preferred_artist,
        &platform_title,
        &platform_artist,
    );
    let music_directory = music_directory()?;
    let directory = music_directory.join(&artist);
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let workspace = DownloadWorkspace::new(&music_directory)?;
    let staged_path = workspace.0.join("track.mp3");
    let output_template = workspace.0.join("track.%(ext)s");

    let mut command = audio_download_command(
        url,
        &output_template,
        &ffmpeg,
        cancel,
        options.cookie_browser.as_deref(),
        None,
    )?;
    let mut output = process_runner::run(
        &mut command,
        Duration::from_secs(30 * 60),
        cancel,
        Some(progress.clone()),
    )?;
    let mut downloaded_from_soundcloud = false;
    if !output.status.success() {
        let error_text = String::from_utf8_lossy(&output.stderr);
        if is_youtube_age_restriction(&error_text) {
            let candidate = crate::music_search::soundcloud_downloadable_match(
                &platform_title,
                &platform_artist,
                cancel,
            )?;
            let Some(candidate) = candidate else {
                return Err(format!(
                    "YouTube a blocat această melodie din cauza restricției de vârstă. Nu am găsit pe SoundCloud o potrivire exactă pentru „{platform_title}” — „{platform_artist}” care să fie disponibilă la descărcare de către creator. Nicio sesiune de browser nu a fost folosită."
                ));
            };
            let mut fallback = audio_download_command(
                &candidate.page_url,
                &output_template,
                &ffmpeg,
                cancel,
                None,
                Some("download"),
            )?;
            output = process_runner::run(
                &mut fallback,
                Duration::from_secs(30 * 60),
                cancel,
                Some(progress.clone()),
            )?;
            if !output.status.success() {
                return Err(format!(
                    "Potrivirea SoundCloud a fost găsită, dar descărcarea a eșuat: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
            downloaded_from_soundcloud = true;
        }
    }
    if let Ok(mut value) = progress.lock() {
        // Validation and library import continue after the transfer completes.
        *value = f32::NAN;
    }
    if !output.status.success() {
        return Err(explain_youtube_auth_error(
            format!(
                "Descărcarea a eșuat: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            options.cookie_browser.as_deref(),
        ));
    }

    // A successful yt-dlp exit alone does not guarantee a valid MP3 was produced.
    // Import this exact file so a failure cannot be hidden by the folder scan.
    process_runner::check_cancel(cancel)?;
    crate::database::validate_audio(&staged_path)?;
    let stem = unique_track_stem(&directory, &title);
    let path = directory.join(format!("{stem}.mp3"));
    std::fs::rename(&staged_path, &path)
        .map_err(|error| format!("Nu am putut salva melodia în Music: {error}"))?;
    let album = options.playlist_album.as_deref().unwrap_or("Descarcate");
    if let Some(playlist_id) = options.playlist_id {
        db.import_downloaded_to_album_and_playlist(&path, album, playlist_id)?;
    } else {
        db.import_downloaded_to_album(&path, album)?;
    }
    let scan = db.scan_downloaded_music(&music_directory)?;
    if let Ok(mut value) = progress.lock() {
        *value = 100.0;
    }
    Ok(format!(
        "Melodia a fost salvată în Music\\{artist}\\{stem}.mp3 și adăugată în albumul {album}{}{}; {} melodii găsite în Music",
        if downloaded_from_soundcloud { " (fallback SoundCloud)" } else { "" },
        options.playlist_id.map(|_| " și playlistului ales").unwrap_or(""),
        scan.discovered
    ))
}

fn audio_download_command(
    url: &str,
    output_template: &std::path::Path,
    ffmpeg: &std::path::Path,
    cancel: &Cancellation,
    browser: Option<&str>,
    format: Option<&str>,
) -> Result<std::process::Command, String> {
    let mut command = crate::yt_dlp::command_with_cookies(cancel, browser)?;
    if let Some(format) = format {
        command.args(["--format", format]);
    }
    command
        .args([
            "--no-warnings",
            "--no-color",
            "--no-playlist",
            "--match-filter",
            "!is_live",
            "--max-filesize",
            "1G",
            "--progress",
            "--newline",
            "--extract-audio",
            "--audio-format",
            "mp3",
            "--embed-metadata",
            "--audio-quality",
            "0",
            "--progress-template",
            "download:__AURA_PROGRESS__%(progress._percent_str)s",
            "--progress-template",
            "postprocess:__AURA_PROCESSING__",
            "--output",
        ])
        .arg(output_template)
        .arg("--ffmpeg-location")
        .arg(ffmpeg)
        .arg("--")
        .arg(url);
    Ok(command)
}

fn is_youtube_age_restriction(error: &str) -> bool {
    error.contains("Sign in to confirm your age")
        || error.contains("age-restricted")
        || error.contains("age restricted")
}

#[cfg(test)]
mod tests {
    #[test]
    fn age_restriction_error_explains_the_browser_cookie_setting() {
        let raw = "ERROR: [youtube] Sign in to confirm your age".to_owned();
        let without_browser = super::explain_youtube_auth_error(raw.clone(), None);
        assert!(without_browser.contains("COOKIE-URI YOUTUBE"));
        let with_browser = super::explain_youtube_auth_error(raw, Some("edge"));
        assert!(with_browser.contains("Microsoft Edge"));
        assert!(with_browser.contains("nu sunt salvate"));
    }

    #[test]
    fn explicit_title_url_validation_and_windows_names_regressions() {
        use super::*;
        assert_eq!(
            track_names("User Custom Title", "", "Platform Song", "Platform Author"),
            ("User Custom Title".into(), "Platform Author".into())
        );
        assert_eq!(
            track_names("", "", "Artist - Song", "Artist"),
            ("Song".into(), "Artist".into())
        );
        assert_eq!(
            track_names("Artist - My Mix", "Artist", "Other", "Other Artist"),
            ("Artist - My Mix".into(), "Artist".into())
        );
        for input in ["COM¹", "LPT²", "COM³.mp3"] {
            assert!(sanitize_filename(input).starts_with('_'));
        }
        assert!(!sanitize_filename(&format!("{} song", "a".repeat(79))).ends_with(' '));
        for input in [
            "http://?",
            "https://#",
            "https://:443",
            "https://user@",
            "https://[::1",
        ] {
            assert!(!is_http_url(input), "{input}");
        }
        for input in [
            "../../file",
            "..\\..\\file",
            "$(whoami)",
            "NUL.mp3",
            "\0\r\n",
            "100%(title)s",
        ] {
            let name = sanitize_filename(input);
            assert_eq!(std::path::Path::new(&name).components().count(), 1);
            assert!(!name.contains(['/', '\\', '%', '\0']));
        }
        assert!(!is_http_url(&format!(
            "https://example.com/{}",
            "x".repeat(8192)
        )));
    }
    use super::{
        find_ffmpeg_directory, is_http_url, sanitize_filename, title_without_author,
        unique_track_stem, DownloadWorkspace,
    };
    use crate::test_support::{wait_for_scans, write_test_mp3, TestLibrary};

    #[test]
    fn enter_submits_search_only_when_the_search_field_has_focus() {
        use eframe::egui;
        let library = TestLibrary::new("search-enter");
        for field in ["download-search-query", "download-url-input"] {
            let ctx = egui::Context::default();
            let frost = crate::load_frost(&ctx);
            let mut downloader = super::Downloader::new();
            let input = |events| egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(850.0, 750.0),
                )),
                events,
                ..Default::default()
            };
            let _ = crate::test_support::run_ui(&ctx, input(vec![]), |ui| {
                downloader.show(ui, &ctx, &frost, &library.db);
            });
            ctx.memory_mut(|memory| memory.request_focus(egui::Id::new(field)));
            let _ = crate::test_support::run_ui(
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
            // Empty-query validation proves submission without contacting a platform.
            assert_eq!(
                downloader.search_error.is_some(),
                field == "download-search-query"
            );
            assert!(!downloader.searching);
        }
    }

    #[test]
    fn complete_download_menu_can_scroll_to_bottom_while_download_is_busy() {
        let library = TestLibrary::new("download-menu-scroll");
        let ctx = eframe::egui::Context::default();
        ctx.set_visuals(crate::dark_visuals());
        let frost = crate::load_frost(&ctx);
        let mut downloader = super::Downloader::new();
        downloader.busy = true;
        *downloader.progress.lock().unwrap() = 37.0;
        downloader.results = (0..10)
            .map(|index| crate::music_search::SearchResult {
                title: format!("Melodie de test {index}"),
                artist: "Artist".to_owned(),
                duration: "3:00".to_owned(),
                page_url: "https://example.com".to_owned(),
                download_url: Some("https://example.com/song".to_owned()),
            })
            .collect();
        let input = || eframe::egui::RawInput {
            screen_rect: Some(eframe::egui::Rect::from_min_size(
                eframe::egui::Pos2::ZERO,
                eframe::egui::vec2(700.0, 280.0),
            )),
            ..Default::default()
        };
        let mut geometry = None;
        let _ = crate::test_support::run_ui(&ctx, input(), |ui| {
            let result = downloader.show(ui, &ctx, &frost, &library.db);
            geometry = Some((
                result.id,
                result.state,
                result.content_size.y,
                result.inner_rect.height(),
            ));
        });
        let (id, mut state, content_height, viewport_height) = geometry.unwrap();
        assert!(content_height > viewport_height + 150.0);
        state.offset.y = content_height;
        state.store(&ctx, id);
        let mut reached_bottom = false;
        let _ = crate::test_support::run_ui(&ctx, input(), |ui| {
            let result = downloader.show(ui, &ctx, &frost, &library.db);
            reached_bottom = result.state.offset.y > 100.0
                && result.state.offset.y + result.inner_rect.height()
                    >= result.content_size.y - 2.0;
        });
        assert!(reached_bottom);
        assert!(downloader.busy);
        assert_eq!(*downloader.progress.lock().unwrap(), 37.0);
    }

    #[test]
    fn portable_ffmpeg_is_found_without_changing_the_system_path() {
        let library = TestLibrary::new("portable-ffmpeg");
        assert!(find_ffmpeg_directory(&library.directory, None).is_none());
        let bin = library.directory.join("ffmpeg").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let suffix = std::env::consts::EXE_SUFFIX;
        std::fs::write(bin.join(format!("ffmpeg{suffix}")), []).unwrap();
        assert!(find_ffmpeg_directory(&library.directory, None).is_none());
        std::fs::write(bin.join(format!("ffprobe{suffix}")), []).unwrap();
        assert_eq!(
            find_ffmpeg_directory(&library.directory, None),
            Some(bin.clone())
        );
        let search_path = std::env::join_paths([&bin]).unwrap();
        assert_eq!(
            find_ffmpeg_directory(&library.music, Some(&search_path)),
            Some(bin)
        );
    }

    #[test]
    fn completed_download_refreshes_the_library_from_every_screen() {
        let library = TestLibrary::new("background-completion");
        let ctx = eframe::egui::Context::default();
        let mut app = crate::AuraApp::with_music_directory(
            library.db.clone(),
            &ctx,
            Ok(library.music.clone()),
        );
        wait_for_scans(&mut app, &ctx);
        let screens = [
            crate::Screen::Home,
            crate::Screen::Songs,
            crate::Screen::Albums,
            crate::Screen::Favorites,
            crate::Screen::Settings,
        ];
        for (index, screen) in screens.into_iter().enumerate() {
            app.navigate(screen);
            let path = library.mp3("Artist", &format!("Song {index}"));
            library.db.import_downloaded(&path).unwrap();
            let message = format!("Completed download {index}");
            app.downloader.busy = true;
            app.downloader.tx.send(Ok(message.clone())).unwrap();
            app.poll_events(&ctx);
            wait_for_scans(&mut app, &ctx);
            assert!(!app.downloader.busy);
            assert_eq!(app.notification.as_ref().unwrap().0, message);
            assert_eq!(app.albums.len(), 1);
            assert_eq!(app.albums[0].track_count, index as i64 + 1);
            if screen != crate::Screen::Favorites {
                assert_eq!(app.page.total, index as i64 + 1);
            }
        }
    }

    #[test]
    fn conversion_files_are_invisible_until_the_download_is_published() {
        let library = TestLibrary::new("staged-download");
        let workspace = DownloadWorkspace::new(&library.music).unwrap();
        let staged = workspace.0.join("track.mp3");
        write_test_mp3(&staged);
        let scan = library.db.scan_downloaded_music(&library.music).unwrap();
        assert_eq!(scan.discovered, 0);
        assert_eq!(library.db.page(0, 10, "", false, None).unwrap().total, 0);

        let directory = library.music.join("Author");
        std::fs::create_dir(&directory).unwrap();
        let destination = directory.join("Song.mp3");
        std::fs::rename(staged, &destination).unwrap();
        library.db.import_downloaded(&destination).unwrap();
        let scan = library.db.scan_downloaded_music(&library.music).unwrap();
        assert_eq!((scan.discovered, scan.updated), (1, 0));
        let track = library
            .db
            .page(0, 10, "", false, None)
            .unwrap()
            .tracks
            .remove(0);
        assert_eq!(track.title, "Song");
        assert_eq!(track.artist, "Author");
        assert_eq!(track.album, "Descarcate");
        let workspace_path = workspace.0.clone();
        drop(workspace);
        assert!(!workspace_path.exists());
    }

    #[test]
    fn filename_replaces_windows_path_characters() {
        assert_eq!(sanitize_filename("artist/song:live?"), "artist_song_live_");
    }

    #[test]
    fn filename_uses_fallback_when_empty() {
        assert_eq!(sanitize_filename("..."), "melodie");
    }

    #[test]
    fn filename_avoids_template_tokens_and_windows_device_names() {
        assert_eq!(sanitize_filename("Song %(title)s"), "Song _(title)s");
        assert_eq!(sanitize_filename("CON"), "_CON");
        assert_eq!(sanitize_filename("LPT2.mp3"), "_LPT2.mp3");
    }

    #[test]
    fn removes_author_from_title_prefix_or_suffix() {
        assert_eq!(title_without_author("Artist - Song", "Artist"), "Song");
        assert_eq!(title_without_author("Song – Artist", "Artist"), "Song");
        assert_eq!(
            title_without_author("Artist - Song", "Other"),
            "Artist - Song"
        );
    }

    #[test]
    fn duplicate_titles_get_a_distinct_filename() {
        let directory =
            std::env::temp_dir().join(format!("aura-title-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("Song.mp3"), []).unwrap();
        assert_eq!(unique_track_stem(&directory, "Song"), "Song (2)");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn accepts_http_urls_without_audio_extensions() {
        assert!(is_http_url("https://youtube.com/watch?v=abc"));
        assert!(is_http_url("https://example.test/stream?token=abc"));
        assert!(!is_http_url("file:///tmp/track.mp3"));
        assert!(!is_http_url("https://"));
    }
}
