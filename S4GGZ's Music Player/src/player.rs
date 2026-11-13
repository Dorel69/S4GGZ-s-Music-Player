//! The worker owns WASAPI, decoders and DB work; the GUI only queues commands.
use crate::audio_analysis::{AnalysisSource, AudioMeter, AudioReactive};
use crate::database::{Database, Track};
use crate::equalizer::{Equalizer, EqualizerSource, SharedEqualizer};
use crate::sorting::TrackSort;
use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink};
use std::{
    collections::HashMap,
    fs::File,
    sync::{mpsc, Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
pub struct AudioPlayer {
    commands: mpsc::SyncSender<Command>,
    status: Arc<Mutex<PlayerStatus>>,
    equalizer: SharedEqualizer,
}
enum Command {
    Play {
        id: i64,
        album: Option<i64>,
        playlist: Option<i64>,
        favorites: bool,
        query: String,
        sort: TrackSort,
    },
    Toggle,
    PlaybackMode(PlaybackMode),
    #[cfg(test)]
    Shuffle(bool),
    #[cfg(test)]
    Repeat(RepeatMode),
    Next,
    Previous,
    Volume(f32),
    Seek(u64),
    #[cfg(test)]
    Reorder(Vec<i64>),
    MoveQueued {
        id: i64,
        movement: QueueMove,
    },
    DeleteTrack {
        id: i64,
        path: String,
        reply: mpsc::Sender<Result<(), String>>,
    },
}

#[derive(Clone, Copy, Debug)]
pub enum QueueMove {
    First,
    Up,
    Down,
    Last,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlaybackMode {
    #[default]
    Off,
    Shuffle,
    Loop,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepeatMode {
    #[default]
    Off,
    #[cfg(test)]
    All,
    One,
}

struct Inner {
    stream: Option<OutputStream>,
    sink: Option<Sink>,
    queue: Vec<i64>,
    normal_queue: Vec<i64>,
    shuffle: bool,
    repeat: RepeatMode,
    shuffle_seed: u64,
    index: usize,
    current: Option<Track>,
    finished: bool,
    volume: f32,
    error: Option<String>,
    meter: Arc<AudioMeter>,
    equalizer: SharedEqualizer,
    #[cfg(test)]
    silent: bool,
}
#[derive(Debug, Clone)]
pub struct PlayerStatus {
    pub current: Option<Track>,
    pub position_ms: u64,
    pub duration_ms: i64,
    pub paused: bool,
    pub playback_mode: PlaybackMode,
    #[cfg(test)]
    pub shuffle: bool,
    #[cfg(test)]
    pub repeat: RepeatMode,
    pub volume: f32,
    pub queue: Vec<i64>,
    pub queue_len: usize,
    pub error: Option<String>,
    pub audio_reactive: AudioReactive,
}
impl Inner {
    fn new() -> Self {
        Self {
            stream: None,
            sink: None,
            queue: vec![],
            normal_queue: vec![],
            shuffle: false,
            repeat: RepeatMode::Off,
            shuffle_seed: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64,
            index: 0,
            current: None,
            finished: false,
            volume: 0.74,
            error: None,
            meter: Arc::new(AudioMeter::default()),
            equalizer: Arc::new(Mutex::new(Equalizer::default())),
            #[cfg(test)]
            silent: false,
        }
    }
    fn connect(&mut self) -> Result<Sink, String> {
        #[cfg(test)]
        if self.silent {
            return Ok(Sink::new().0);
        }
        if self.stream.is_none() {
            self.stream = Some(
                OutputStreamBuilder::open_default_stream()
                    .map_err(|error| format!("Dispozitiv audio indisponibil: {error}"))?,
            );
        }
        Ok(Sink::connect_new(
            self.stream
                .as_ref()
                .ok_or("Ieșire audio indisponibilă")?
                .mixer(),
        ))
    }
    fn start(&mut self, db: &Database, from: usize) -> Result<(), String> {
        self.start_range(db, from, self.queue.len())
    }

    fn set_shuffle(&mut self, enabled: bool) {
        if self.shuffle == enabled {
            return;
        }
        let start = self.index.saturating_add(1).min(self.queue.len());
        if enabled {
            self.normal_queue = self.queue.clone();
            shuffle_tracks(&mut self.queue[start..], &mut self.shuffle_seed);
        } else {
            let ranks: HashMap<i64, usize> = self
                .normal_queue
                .iter()
                .enumerate()
                .map(|(rank, id)| (*id, rank))
                .collect();
            self.queue[start..].sort_by_key(|id| ranks.get(id).copied().unwrap_or(usize::MAX));
        }
        self.shuffle = enabled;
    }

    fn start_range(&mut self, db: &Database, from: usize, end: usize) -> Result<(), String> {
        let mut last_error = "Nu mai există piese redabile în coadă".to_owned();
        for index in from..end.min(self.queue.len()) {
            let Some(track) = db.get(self.queue[index])? else {
                continue;
            };
            if track.is_missing {
                continue;
            }
            if track.file_path.to_ascii_lowercase().ends_with(".opus") {
                last_error =
                    "Formatul OPUS nu este suportat. Convertește piesa în FLAC, WAV sau MP3."
                        .into();
                continue;
            }
            let file = match File::open(&track.file_path) {
                Ok(file) => file,
                Err(error) => {
                    if error.kind() == std::io::ErrorKind::NotFound {
                        db.missing(track.id);
                    }
                    last_error = format!("Fișier inaccesibil ({}): {error}", track.title);
                    continue;
                }
            };
            let decoded = match Decoder::try_from(file) {
                Ok(decoded) => decoded,
                Err(error) => {
                    last_error = format!("Fișier invalid ({}): {error}", track.title);
                    continue;
                }
            };
            // Prepare the destination before stopping the current, playable track.
            let sink = self.connect()?;
            let meter = Arc::new(AudioMeter::default());
            sink.set_volume(self.volume);
            sink.pause();
            let equalized = EqualizerSource::new(decoded, self.equalizer.clone())?;
            sink.append(AnalysisSource::new(equalized, meter.clone()));
            if let Some(previous) = self.sink.replace(sink) {
                previous.stop();
            }
            self.meter = meter;
            self.index = index;
            self.current = Some(track);
            self.finished = false;
            self.error = None;
            if let Some(sink) = &self.sink {
                sink.play();
            }
            db.note_play(self.queue[index]);
            return Ok(());
        }
        Err(last_error)
    }
    fn tick(&mut self, db: &Database) {
        if !self.finished
            && self.current.is_some()
            && self
                .sink
                .as_ref()
                .is_some_and(|sink| sink.empty() && !sink.is_paused())
        {
            let next = self.index.saturating_add(1);
            let result = match self.repeat {
                RepeatMode::One => self.start_range(db, self.index, next),
                #[cfg(test)]
                RepeatMode::All => self.start(db, next).or_else(|_| self.start(db, 0)),
                RepeatMode::Off if next >= self.queue.len() => {
                    self.finished = true;
                    return;
                }
                RepeatMode::Off => self.start(db, next),
            };
            if let Err(error) = result {
                self.finished = true;
                self.error = Some(error);
            }
        }
    }
    fn handle(&mut self, db: &Database, command: Command) -> Result<(), String> {
        self.error = None;
        match command {
            Command::Play {
                id,
                album,
                playlist,
                favorites,
                query,
                sort,
            } => {
                let mut queue = if let Some(playlist_id) = playlist {
                    db.playlist_order_sorted(playlist_id, &query, sort)?
                } else {
                    db.order_sorted(album, favorites, &query, sort)?
                };
                if !queue.contains(&id) {
                    queue.insert(0, id);
                }
                let normal_queue = queue.clone();
                let mut index = queue.iter().position(|entry| *entry == id).unwrap_or(0);
                if self.shuffle {
                    // A selected track always starts first; shuffle includes the
                    // whole filtered collection, including earlier sorted rows.
                    queue.remove(index);
                    queue.insert(0, id);
                    shuffle_tracks(&mut queue[1..], &mut self.shuffle_seed);
                    index = 0;
                }
                let previous = std::mem::replace(&mut self.queue, queue);
                if let Err(error) = self.start(db, index) {
                    self.queue = previous;
                    return Err(error);
                }
                self.normal_queue = normal_queue;
            }
            Command::PlaybackMode(mode) => {
                // Publish only after this command returns: the UI never sees
                // an intermediate combination of Shuffle and Loop.
                self.set_shuffle(mode == PlaybackMode::Shuffle);
                self.repeat = match mode {
                    PlaybackMode::Shuffle => RepeatMode::Off,
                    PlaybackMode::Loop => RepeatMode::One,
                    PlaybackMode::Off => RepeatMode::Off,
                };
            }
            #[cfg(test)]
            Command::Shuffle(enabled) => self.set_shuffle(enabled),
            #[cfg(test)]
            Command::Repeat(mode) => self.repeat = mode,
            Command::Toggle => {
                if self.current.is_none() {
                    return Err("Selectează o piesă din bibliotecă".into());
                }
                if self.finished || self.sink.as_ref().is_none_or(|sink| sink.empty()) {
                    self.start(db, self.index)?;
                } else if let Some(sink) = &self.sink {
                    if sink.is_paused() {
                        sink.play()
                    } else {
                        sink.pause()
                    }
                }
            }
            Command::Next => {
                let next = self.index.saturating_add(1);
                #[cfg(test)]
                if self.repeat == RepeatMode::All {
                    self.start(db, next).or_else(|_| self.start(db, 0))?;
                    return Ok(());
                }
                self.start(db, next)?;
            }
            Command::Previous => {
                if self.finished {
                    self.start(db, self.index)?;
                } else if self
                    .sink
                    .as_ref()
                    .is_some_and(|sink| sink.get_pos().as_secs() >= 3)
                {
                    if let Some(sink) = &self.sink {
                        sink.try_seek(Duration::ZERO)
                            .map_err(|error| error.to_string())?;
                    }
                } else {
                    self.start(db, self.index.saturating_sub(1))?;
                }
            }
            Command::Volume(volume) => {
                if !volume.is_finite() {
                    return Err("Volumul trebuie să fie un număr finit".into());
                }
                self.volume = volume.clamp(0.0, 1.0);
                if let Some(sink) = &self.sink {
                    sink.set_volume(self.volume);
                }
            }
            Command::Seek(ms) => {
                if self.finished {
                    self.start(db, self.index)?;
                }
                if let Some(sink) = &self.sink {
                    let limit = self
                        .current
                        .as_ref()
                        .map_or(0, |track| track.duration_ms.max(0) as u64);
                    sink.try_seek(Duration::from_millis(ms.min(limit)))
                        .map_err(|error| error.to_string())?;
                }
            }
            #[cfg(test)]
            Command::Reorder(ids) => {
                let start = self.index.saturating_add(1);
                let mut before = self
                    .queue
                    .iter()
                    .skip(start)
                    .take(ids.len())
                    .copied()
                    .collect::<Vec<_>>();
                let mut after = ids.clone();
                before.sort_unstable();
                after.sort_unstable();
                if before != after {
                    return Err("Coada a fost modificată între timp".into());
                }
                if !ids.is_empty() {
                    self.queue.splice(start..start + ids.len(), ids);
                }
            }
            Command::MoveQueued { id, movement } => {
                let start = self.index.saturating_add(1);
                let from = self
                    .queue
                    .iter()
                    .enumerate()
                    .skip(start)
                    .find_map(|(index, entry)| (*entry == id).then_some(index))
                    .ok_or("Piesa nu mai este în lista de așteptare")?;
                let to = match movement {
                    QueueMove::First => start,
                    QueueMove::Up => from.saturating_sub(1).max(start),
                    QueueMove::Down => (from + 1).min(self.queue.len() - 1),
                    QueueMove::Last => self.queue.len() - 1,
                };
                let entry = self.queue.remove(from);
                self.queue.insert(to, entry);
            }
            Command::DeleteTrack { id, path, reply } => {
                // Keep filesystem work off the UI and the sample callback.
                let result = self.delete_track(db, id, &path);
                let _ = reply.send(result);
            }
        }
        Ok(())
    }

    fn delete_track(&mut self, db: &Database, id: i64, expected_path: &str) -> Result<(), String> {
        let track = db.get(id)?.ok_or("Piesa nu mai există în bibliotecă")?;
        if track.file_path != expected_path {
            return Err("Fișierul piesei s-a schimbat; reîncearcă din lista actualizată".into());
        }
        let current_id = self.current.as_ref().map(|track| track.id);
        let deleting_current = current_id == Some(id);
        let was_paused = self.sink.as_ref().is_none_or(|sink| sink.is_paused()) || self.finished;
        let position = self.sink.as_ref().map_or(Duration::ZERO, Sink::get_pos);
        if deleting_current {
            if let Some(sink) = self.sink.take() {
                sink.stop();
            }
            // On Windows an active decoder owns an open file. Dropping the
            // stream after its Sink releases that decoder before unlinking it.
            self.stream = None;
            self.current = None;
            self.finished = false;
            self.meter = Arc::new(AudioMeter::default());
        }
        if let Err(error) = db.delete_audio_file(id, expected_path) {
            if deleting_current && self.start(db, self.index).is_ok() {
                if let Some(sink) = &self.sink {
                    let _ = sink.try_seek(position);
                    if was_paused {
                        sink.pause();
                    }
                }
            }
            return Err(error);
        }
        let old_index = self.index;
        self.queue.retain(|entry| *entry != id);
        self.normal_queue.retain(|entry| *entry != id);
        if deleting_current {
            self.index = old_index.min(self.queue.len());
            if self.index < self.queue.len() {
                match self.start(db, self.index) {
                    Ok(()) => {
                        if was_paused {
                            if let Some(sink) = &self.sink {
                                sink.pause();
                            }
                        }
                    }
                    Err(error) => {
                        self.finished = true;
                        self.error = Some(format!(
                            "Piesa a fost ștearsă. Următoarea piesă nu poate fi redată: {error}"
                        ));
                    }
                }
            }
        } else if let Some(current) = current_id {
            self.index = self
                .queue
                .iter()
                .position(|entry| *entry == current)
                .unwrap_or(0);
        }
        Ok(())
    }
    fn snapshot(&self) -> PlayerStatus {
        let paused = self.finished
            || self.current.is_none()
            || self
                .sink
                .as_ref()
                .is_none_or(|sink| sink.is_paused() || sink.empty());
        let duration_ms = self.current.as_ref().map_or(0, |track| track.duration_ms);
        PlayerStatus {
            current: self.current.clone(),
            position_ms: if self.finished {
                duration_ms.max(0) as u64
            } else {
                self.sink
                    .as_ref()
                    .map_or(0, |sink| sink.get_pos().as_millis() as u64)
            },
            duration_ms,
            paused,
            playback_mode: if self.repeat == RepeatMode::One {
                PlaybackMode::Loop
            } else if self.shuffle {
                PlaybackMode::Shuffle
            } else {
                PlaybackMode::Off
            },
            #[cfg(test)]
            shuffle: self.shuffle,
            #[cfg(test)]
            repeat: self.repeat,
            volume: self.volume,
            queue: self
                .queue
                .iter()
                .skip(self.index.saturating_add(1))
                .take(120)
                .copied()
                .collect(),
            queue_len: self
                .queue
                .len()
                .saturating_sub(self.index.saturating_add(1)),
            error: self.error.clone(),
            audio_reactive: if paused {
                AudioReactive::default()
            } else {
                self.meter.snapshot()
            },
        }
    }
}
impl AudioPlayer {
    pub fn new(db: Database) -> Self {
        let (commands, receiver) = mpsc::sync_channel(128);
        let status = Arc::new(Mutex::new(Inner::new().snapshot()));
        let published = status.clone();
        let equalizer = Arc::new(Mutex::new(Equalizer::default()));
        let shared_equalizer = equalizer.clone();
        std::thread::spawn(move || {
            // WASAPI is opened and dropped on this thread, including shutdown.
            let mut engine = Inner::new();
            engine.equalizer = shared_equalizer;
            loop {
                match receiver.recv_timeout(Duration::from_millis(40)) {
                    Ok(command) => {
                        if let Err(error) = engine.handle(&db, command) {
                            engine.error = Some(error);
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                engine.tick(&db);
                if let Ok(mut status) = published.lock() {
                    *status = engine.snapshot();
                }
            }
        });
        Self {
            commands,
            status,
            equalizer,
        }
    }
    /// Publishing is independent of the bounded playback command queue, so
    /// fast slider drags coalesce to the latest complete configuration.
    pub fn set_equalizer(&self, settings: Equalizer) -> Result<(), String> {
        settings.validate()?;
        *self
            .equalizer
            .lock()
            .map_err(|_| "Starea egalizatorului nu este disponibilă")? = settings;
        Ok(())
    }
    fn send(&self, command: Command) -> Result<(), String> {
        self.commands
            .try_send(command)
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => {
                    "Motorul audio este ocupat; încearcă din nou peste câteva secunde".into()
                }
                mpsc::TrySendError::Disconnected(_) => {
                    "Motorul audio nu mai este disponibil".into()
                }
            })
    }
    pub fn play_sorted(
        &self,
        _db: &Database,
        id: i64,
        album_id: Option<i64>,
        favorites: bool,
        query: &str,
        sort: TrackSort,
    ) -> Result<(), String> {
        self.send(Command::Play {
            id,
            album: album_id,
            playlist: None,
            favorites,
            query: query.to_owned(),
            sort,
        })
    }
    pub fn play_playlist_sorted(
        &self,
        id: i64,
        playlist_id: i64,
        query: &str,
        sort: TrackSort,
    ) -> Result<(), String> {
        self.send(Command::Play {
            id,
            album: None,
            playlist: Some(playlist_id),
            favorites: false,
            query: query.to_owned(),
            sort,
        })
    }
    pub fn status(&self) -> Result<PlayerStatus, String> {
        self.status
            .lock()
            .map(|status| status.clone())
            .map_err(|error| error.to_string())
    }
    pub fn toggle(&self) -> Result<(), String> {
        self.send(Command::Toggle)
    }
    pub fn set_playback_mode(&self, mode: PlaybackMode) -> Result<(), String> {
        self.send(Command::PlaybackMode(mode))
    }
    pub fn volume(&self, volume: f32) -> Result<(), String> {
        if !volume.is_finite() {
            return Err("Volumul trebuie să fie un număr finit".into());
        }
        self.send(Command::Volume(volume))
    }
    pub fn seek(&self, ms: u64) -> Result<(), String> {
        self.send(Command::Seek(ms))
    }
    pub fn next(&self, _db: &Database) -> Result<(), String> {
        self.send(Command::Next)
    }
    pub fn previous(&self, _db: &Database) -> Result<(), String> {
        self.send(Command::Previous)
    }
    pub fn move_queued(&self, id: i64, movement: QueueMove) -> Result<(), String> {
        self.send(Command::MoveQueued { id, movement })
    }
    pub fn delete_track(
        &self,
        id: i64,
        path: String,
    ) -> Result<mpsc::Receiver<Result<(), String>>, String> {
        let (reply, receiver) = mpsc::channel();
        self.send(Command::DeleteTrack { id, path, reply })?;
        Ok(receiver)
    }
}

/// Fisher–Yates over the complete upcoming queue. SplitMix64 supplies local
/// randomness without sharing state with audio processing or adding a dependency.
fn shuffle_tracks(tracks: &mut [i64], seed: &mut u64) {
    for index in (1..tracks.len()).rev() {
        let range = index as u64 + 1;
        let threshold = range.wrapping_neg() % range;
        let random = loop {
            *seed = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut value = *seed;
            value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            value ^= value >> 31;
            if value >= threshold {
                break value;
            }
        };
        tracks.swap(index, (random % range) as usize);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestLibrary;

    #[test]
    fn playback_mode_switches_are_exclusive_and_preserve_paused_playback_and_history() {
        assert_eq!(PlaybackMode::default(), PlaybackMode::Off);
        let library = TestLibrary::new("exclusive-playback-modes");
        for title in ["A", "B", "C", "D", "E", "F", "G"] {
            library.mp3("Artist", title);
        }
        library.db.scan_downloaded_music(&library.music).unwrap();
        let mut engine = Inner::new();
        engine.silent = true;
        engine.shuffle_seed = 17;
        assert_eq!(engine.snapshot().playback_mode, PlaybackMode::Off);
        assert!(!engine.shuffle);
        assert_eq!(engine.repeat, RepeatMode::Off);
        let ordered = library
            .db
            .order_sorted(None, false, "", TrackSort::default())
            .unwrap();
        engine
            .handle(
                &library.db,
                Command::Play {
                    id: ordered[0],
                    album: None,
                    playlist: None,
                    favorites: false,
                    query: String::new(),
                    sort: TrackSort::default(),
                },
            )
            .unwrap();
        engine.start(&library.db, 2).unwrap();
        engine.sink.as_ref().unwrap().pause();
        let selected = engine.current.as_ref().unwrap().id;
        let history = engine.queue[..=engine.index].to_vec();
        let position = engine.sink.as_ref().unwrap().get_pos();
        let meter = engine.meter.clone();
        for mode in [
            PlaybackMode::Loop,
            PlaybackMode::Off,
            PlaybackMode::Shuffle,
            PlaybackMode::Off,
            PlaybackMode::Loop,
        ] {
            engine
                .handle(&library.db, Command::PlaybackMode(mode))
                .unwrap();
            let status = engine.snapshot();
            assert_eq!(status.playback_mode, mode);
            assert_eq!(status.shuffle, mode == PlaybackMode::Shuffle);
            assert_eq!(
                status.repeat,
                if mode == PlaybackMode::Loop {
                    RepeatMode::One
                } else {
                    RepeatMode::Off
                }
            );
            assert!(engine.sink.as_ref().unwrap().is_paused());
            assert!(!engine.sink.as_ref().unwrap().empty());
            assert_eq!(engine.sink.as_ref().unwrap().get_pos(), position);
            assert!(
                Arc::ptr_eq(&engine.meter, &meter),
                "Switching must not restart the decoder"
            );
            assert_eq!(engine.current.as_ref().unwrap().id, selected);
            assert_eq!(&engine.queue[..=engine.index], history);
            if mode == PlaybackMode::Loop {
                let upcoming = &engine.queue[engine.index + 1..];
                let expected = ordered
                    .iter()
                    .copied()
                    .filter(|id| !history.contains(id))
                    .collect::<Vec<_>>();
                assert_eq!(upcoming, expected);
            }
            let stable_queue = engine.queue.clone();
            engine
                .handle(&library.db, Command::PlaybackMode(mode))
                .unwrap();
            assert_eq!(
                engine.queue, stable_queue,
                "Selecting the current mode must be idempotent"
            );
        }
    }

    #[test]
    fn starting_a_track_from_a_playlist_queues_only_that_playlists_members() {
        let library = TestLibrary::new("playlist-playback");
        for title in ["A", "B", "Outside"] {
            library.mp3("Artist", title);
        }
        library.db.scan_downloaded_music(&library.music).unwrap();
        let tracks = library.db.page(0, 10, "", false, None).unwrap().tracks;
        let playlist = library.db.create_playlist("Road Mix").unwrap();
        library
            .db
            .add_track_to_playlist(playlist, tracks[0].id)
            .unwrap();
        library
            .db
            .add_track_to_playlist(playlist, tracks[1].id)
            .unwrap();
        let mut engine = Inner::new();
        engine.silent = true;
        engine
            .handle(
                &library.db,
                Command::Play {
                    id: tracks[1].id,
                    album: None,
                    playlist: Some(playlist),
                    favorites: false,
                    query: String::new(),
                    sort: TrackSort::default(),
                },
            )
            .unwrap();
        assert_eq!(engine.queue, vec![tracks[0].id, tracks[1].id]);
        assert_eq!(engine.current.as_ref().unwrap().id, tracks[1].id);
    }

    #[test]
    fn playback_mode_loop_repeats_the_current_track_and_shuffle_exits_the_loop() {
        let library = TestLibrary::new("repeat-one");
        for title in ["A", "B"] {
            library.mp3("Artist", title);
        }
        library.db.scan_downloaded_music(&library.music).unwrap();
        let mut engine = Inner::new();
        engine.silent = true;
        let order = library
            .db
            .order_sorted(None, false, "", TrackSort::default())
            .unwrap();
        engine
            .handle(&library.db, Command::PlaybackMode(PlaybackMode::Loop))
            .unwrap();
        engine
            .handle(
                &library.db,
                Command::Play {
                    id: order[0],
                    album: None,
                    playlist: None,
                    favorites: false,
                    query: String::new(),
                    sort: TrackSort::default(),
                },
            )
            .unwrap();
        engine.sink = Some(Sink::new().0);
        engine.tick(&library.db);
        assert_eq!(engine.current.as_ref().unwrap().id, order[0]);
        assert_eq!(engine.index, 0);
        assert!(!engine.finished);
        assert!(!engine.sink.as_ref().unwrap().empty());
        engine.handle(&library.db, Command::Next).unwrap();
        assert_eq!(engine.current.as_ref().unwrap().id, order[1]);
        assert!(engine.handle(&library.db, Command::Next).is_err());
        assert_eq!(engine.current.as_ref().unwrap().id, order[1]);
        assert!(!engine.sink.as_ref().unwrap().empty());
        assert_eq!(engine.queue, order);
        engine
            .handle(&library.db, Command::PlaybackMode(PlaybackMode::Shuffle))
            .unwrap();
        engine.sink = Some(Sink::new().0);
        engine.tick(&library.db);
        assert!(
            engine.finished,
            "Shuffle must disable the single-track loop"
        );
        assert_eq!(engine.current.as_ref().unwrap().id, order[1]);
    }

    #[test]
    fn repeat_all_wraps_manual_and_automatic_advancement_past_unplayable_rows() {
        let library = TestLibrary::new("repeat-all");
        for title in ["A", "B"] {
            library.mp3("Artist", title);
        }
        library.db.scan_downloaded_music(&library.music).unwrap();
        let ids = library
            .db
            .order_sorted(None, false, "", TrackSort::default())
            .unwrap();
        let mut engine = Inner::new();
        engine.silent = true;
        engine.shuffle = false;
        engine.queue = vec![ids[0], i64::MAX, ids[1], i64::MAX - 1];
        engine.handle(&library.db, Command::Shuffle(true)).unwrap();
        let order = engine.queue.clone();
        // Start at the last actual track, regardless of its shuffled position.
        let last = engine
            .queue
            .iter()
            .rposition(|id| ids.contains(id))
            .unwrap();
        engine.start(&library.db, last).unwrap();
        engine
            .handle(&library.db, Command::Repeat(RepeatMode::All))
            .unwrap();
        engine.sink = Some(Sink::new().0);
        engine.tick(&library.db);
        let first = engine.queue.iter().position(|id| ids.contains(id)).unwrap();
        assert_eq!(engine.index, first);
        assert!(!engine.finished);
        assert!(engine.error.is_none());
        engine.start(&library.db, last).unwrap();
        engine.handle(&library.db, Command::Next).unwrap();
        assert_eq!(engine.index, first);
        assert_eq!(
            engine.queue, order,
            "A repeated shuffle cycle retains one copy of each row"
        );
    }

    #[test]
    fn repeat_failures_stop_once_and_repeat_one_does_not_skip_the_selected_track() {
        for mode in [RepeatMode::One, RepeatMode::All] {
            let library = TestLibrary::new("repeat-unplayable");
            let first = library.mp3("Artist", "A");
            let second = library.mp3("Artist", "B");
            library.db.scan_downloaded_music(&library.music).unwrap();
            let mut engine = Inner::new();
            engine.silent = true;
            engine.queue = library
                .db
                .order_sorted(None, false, "", TrackSort::default())
                .unwrap();
            engine.start(&library.db, 0).unwrap();
            engine.handle(&library.db, Command::Repeat(mode)).unwrap();
            let selected = engine.current.as_ref().unwrap().id;
            engine.sink = Some(Sink::new().0);
            std::fs::remove_file(first).unwrap();
            if mode == RepeatMode::All {
                std::fs::remove_file(second).unwrap();
            }
            engine.tick(&library.db);
            assert!(engine.finished);
            assert!(engine.error.is_some());
            assert_eq!(engine.current.as_ref().unwrap().id, selected);
            assert!(engine.snapshot().paused);
            engine.error = Some("Stopped after a failed cycle".into());
            engine.tick(&library.db);
            assert_eq!(
                engine.error.as_deref(),
                Some("Stopped after a failed cycle")
            );
        }
    }

    #[test]
    fn shuffle_covers_the_full_queue_and_restores_only_unplayed_tracks() {
        let library = TestLibrary::new("shuffle-full-queue");
        let mut engine = Inner::new();
        engine.shuffle = false;
        engine.shuffle_seed = 0;
        engine.queue = (1..=200).collect();
        engine.index = 2;
        engine.handle(&library.db, Command::Shuffle(true)).unwrap();
        assert!(engine.snapshot().shuffle);
        assert_eq!(&engine.queue[..3], &[1, 2, 3]);
        assert_ne!(&engine.queue[123..], &(124..=200).collect::<Vec<_>>());
        let mut upcoming = engine.queue[3..].to_vec();
        upcoming.sort_unstable();
        assert_eq!(upcoming, (4..=200).collect::<Vec<_>>());
        let shuffled = engine.queue.clone();
        engine.handle(&library.db, Command::Shuffle(true)).unwrap();
        assert_eq!(
            engine.queue, shuffled,
            "Setting the same mode is idempotent"
        );

        // Playback has consumed part of the shuffled order before toggling off.
        engine.index = 10;
        let history = engine.queue[..=engine.index].to_vec();
        let mut remaining = engine.queue[engine.index + 1..].to_vec();
        remaining.sort_unstable();
        engine.handle(&library.db, Command::Shuffle(false)).unwrap();
        assert!(!engine.snapshot().shuffle);
        assert_eq!(&engine.queue[..=engine.index], history);
        assert_eq!(&engine.queue[engine.index + 1..], remaining);
    }

    #[test]
    fn shuffled_selection_starts_the_chosen_track_and_respects_filter_and_sort() {
        let library = TestLibrary::new("shuffle-selection");
        for title in ["A", "B", "C", "D", "E", "F"] {
            library.mp3("Matching artist", title);
        }
        library.mp3("Other artist", "Other song");
        library.db.scan_downloaded_music(&library.music).unwrap();
        let sort = TrackSort {
            column: crate::sorting::SortColumn::Title,
            descending: true,
        };
        let ordered = library
            .db
            .order_sorted(None, false, "Matching", sort)
            .unwrap();
        assert_eq!(ordered.len(), 6);
        let selected = ordered[3];
        let mut engine = Inner::new();
        engine.silent = true;
        engine.shuffle_seed = 7;
        engine.handle(&library.db, Command::Shuffle(true)).unwrap();
        engine
            .handle(
                &library.db,
                Command::Play {
                    id: selected,
                    album: None,
                    playlist: None,
                    favorites: false,
                    query: "Matching".into(),
                    sort,
                },
            )
            .unwrap();
        assert_eq!(engine.current.as_ref().unwrap().id, selected);
        assert_eq!(engine.index, 0);
        let mut all = engine.queue.clone();
        all.sort_unstable();
        let mut expected = ordered.clone();
        expected.sort_unstable();
        assert_eq!(all, expected, "Each filtered track must occur exactly once");

        engine.sink.as_ref().unwrap().pause();
        engine.handle(&library.db, Command::Shuffle(false)).unwrap();
        assert!(engine.sink.as_ref().unwrap().is_paused());
        assert!(!engine.sink.as_ref().unwrap().empty());
        assert_eq!(engine.current.as_ref().unwrap().id, selected);
        assert_eq!(engine.queue[0], selected);
        assert_eq!(
            &engine.queue[1..],
            ordered
                .into_iter()
                .filter(|id| *id != selected)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn shuffle_allows_manual_moves_and_does_not_restore_deleted_tracks() {
        let library = TestLibrary::new("shuffle-delete-move");
        for title in ["A", "B", "C", "D", "E", "F"] {
            library.mp3("Artist", title);
        }
        library.db.scan_downloaded_music(&library.music).unwrap();
        let ordered = library
            .db
            .order_sorted(None, false, "", TrackSort::default())
            .unwrap();
        let mut engine = Inner::new();
        engine.silent = true;
        engine.shuffle = false;
        engine.queue = ordered.clone();
        engine.start(&library.db, 0).unwrap();
        engine.sink.as_ref().unwrap().pause();
        engine.handle(&library.db, Command::Shuffle(true)).unwrap();
        let moved = engine.queue[1];
        engine
            .handle(
                &library.db,
                Command::MoveQueued {
                    id: moved,
                    movement: QueueMove::Last,
                },
            )
            .unwrap();
        assert_eq!(engine.queue.last(), Some(&moved));
        let deleted = library.db.get(engine.queue[2]).unwrap().unwrap();
        engine
            .delete_track(&library.db, deleted.id, &deleted.file_path)
            .unwrap();
        engine.handle(&library.db, Command::Shuffle(false)).unwrap();
        assert_eq!(
            engine.queue,
            ordered
                .into_iter()
                .filter(|id| *id != deleted.id)
                .collect::<Vec<_>>()
        );
        assert!(engine.sink.as_ref().unwrap().is_paused());
        assert!(!std::path::Path::new(&deleted.file_path).exists());
    }

    #[test]
    fn shuffle_handles_an_empty_queue_and_a_single_track() {
        let library = TestLibrary::new("shuffle-short-queues");
        let mut engine = Inner::new();
        engine.shuffle = false;
        for queue in [vec![], vec![1]] {
            engine.queue = queue.clone();
            engine.handle(&library.db, Command::Shuffle(true)).unwrap();
            engine.handle(&library.db, Command::Shuffle(false)).unwrap();
            assert_eq!(engine.queue, queue);
        }
    }

    #[test]
    fn end_of_queue_retains_selection_and_toggle_restarts() {
        let library = TestLibrary::new("player-end");
        library.mp3("Artist", "Silence");
        library.db.scan_downloaded_music(&library.music).unwrap();
        let track = library
            .db
            .page(0, 20, "", false, None)
            .unwrap()
            .tracks
            .remove(0);
        let mut engine = Inner::new();
        engine.silent = true;
        engine.queue = vec![track.id];
        engine.current = Some(track.clone());
        engine.sink = Some(Sink::new().0);
        engine.tick(&library.db);
        assert!(engine.finished);
        assert_eq!(engine.current.as_ref().unwrap().id, track.id);
        engine.handle(&library.db, Command::Toggle).unwrap();
        assert!(!engine.finished);
        assert!(!engine.sink.as_ref().unwrap().empty());
        assert!(engine.handle(&library.db, Command::Next).is_err());
        assert_eq!(engine.current.as_ref().unwrap().id, track.id);
        assert!(!engine.sink.as_ref().unwrap().empty());
    }
    #[test]
    fn invalid_volume_is_rejected_without_changing_state() {
        let library = TestLibrary::new("player-volume");
        let mut engine = Inner::new();
        assert!(engine
            .handle(&library.db, Command::Volume(f32::NAN))
            .is_err());
        assert_eq!(engine.volume, 0.74);
    }

    #[test]
    fn equalizer_updates_coalesce_and_invalid_settings_leave_audio_unchanged() {
        let library = TestLibrary::new("player-equalizer");
        let player = AudioPlayer::new(library.db.clone());
        let expected = crate::equalizer::Preset::Vocal.settings();
        for preset in crate::equalizer::Preset::ALL {
            player.set_equalizer(preset.settings()).unwrap();
        }
        player.set_equalizer(expected).unwrap();
        assert_eq!(*player.equalizer.lock().unwrap(), expected);
        let mut invalid = expected;
        invalid.preamp_db = f32::NAN;
        assert!(player.set_equalizer(invalid).is_err());
        invalid = expected;
        invalid.bands[0].gain_db = 13.;
        assert!(player.set_equalizer(invalid).is_err());
        assert_eq!(*player.equalizer.lock().unwrap(), expected);
    }

    #[test]
    fn queue_reordering_changes_only_upcoming_tracks_and_rejects_stale_orders() {
        let library = TestLibrary::new("queue-arrows");
        let mut engine = Inner::new();
        engine.queue = vec![1, 2, 3, 4];
        engine.index = 1;
        engine
            .handle(&library.db, Command::Reorder(vec![4, 3]))
            .unwrap();
        assert_eq!(engine.queue, [1, 2, 4, 3]);
        assert_eq!(engine.index, 1);
        assert!(engine
            .handle(&library.db, Command::Reorder(vec![9, 3]))
            .is_err());
        assert_eq!(engine.queue, [1, 2, 4, 3]);
    }

    #[test]
    fn queue_jump_moves_reach_the_actual_ends_beyond_the_visible_120_tracks() {
        let library = TestLibrary::new("queue-ends");
        let mut engine = Inner::new();
        engine.queue = (1..=200).collect();
        engine.index = 2;
        engine
            .handle(
                &library.db,
                Command::MoveQueued {
                    id: 4,
                    movement: QueueMove::Last,
                },
            )
            .unwrap();
        assert_eq!(engine.queue.last(), Some(&4));
        assert_eq!(engine.snapshot().queue_len, 197);
        engine
            .handle(
                &library.db,
                Command::MoveQueued {
                    id: 4,
                    movement: QueueMove::First,
                },
            )
            .unwrap();
        assert_eq!(engine.queue[3], 4);
        engine
            .handle(
                &library.db,
                Command::MoveQueued {
                    id: 4,
                    movement: QueueMove::Down,
                },
            )
            .unwrap();
        assert_eq!(engine.queue[4], 4);
        engine
            .handle(
                &library.db,
                Command::MoveQueued {
                    id: 4,
                    movement: QueueMove::Up,
                },
            )
            .unwrap();
        assert_eq!(engine.queue[3], 4);
        assert!(engine
            .handle(
                &library.db,
                Command::MoveQueued {
                    id: 3,
                    movement: QueueMove::First
                }
            )
            .is_err());
        assert_eq!(&engine.queue[..3], &[1, 2, 3]);
    }

    #[test]
    fn deleting_the_current_file_releases_it_and_preserves_pause_for_the_next_track() {
        for paused in [false, true] {
            let library = TestLibrary::new("delete-current");
            let selected_path = library.mp3("Artist", "A");
            let next_path = library.mp3("Artist", "B");
            library.db.scan_downloaded_music(&library.music).unwrap();
            let tracks = library.db.page(0, 20, "", false, None).unwrap().tracks;
            let mut engine = Inner::new();
            engine.silent = true;
            engine.queue = tracks.iter().map(|track| track.id).collect();
            engine.start(&library.db, 0).unwrap();
            let selected = engine.current.clone().unwrap();
            if paused {
                engine.sink.as_ref().unwrap().pause();
            }
            engine
                .delete_track(&library.db, selected.id, &selected.file_path)
                .unwrap();
            assert!(!selected_path.exists());
            assert!(next_path.exists());
            assert_eq!(engine.current.as_ref().unwrap().id, tracks[1].id);
            assert_eq!(engine.sink.as_ref().unwrap().is_paused(), paused);
            assert_eq!(engine.queue, [tracks[1].id]);
        }
    }
}
