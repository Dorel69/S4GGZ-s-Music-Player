use crate::sorting::{self, TrackSort};
use lofty::{
    file::{AudioFile, TaggedFileExt},
    tag::Accessor,
};
use rayon::prelude::*;
use rusqlite::functions::FunctionFlags;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::UNIX_EPOCH,
};
use walkdir::WalkDir;

#[derive(Clone)]
pub struct Database {
    path: Arc<PathBuf>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: i64,
    pub file_path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_id: i64,
    pub duration_ms: i64,
    pub bitrate: Option<i64>,
    pub sample_rate: Option<i64>,
    pub genre: String,
    pub track_number: Option<i64>,
    pub is_favorite: bool,
    pub is_missing: bool,
    pub play_count: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub total: i64,
    pub tracks: Vec<Track>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    pub id: i64,
    pub title: String,
    pub artist: String,
    pub track_count: i64,
    pub duration_ms: i64,
    pub representative_track: i64,
    pub representative_path: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: i64,
    pub title: String,
    pub track_count: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanStats {
    pub discovered: usize,
    pub updated: usize,
    pub missing: usize,
    pub errors: usize,
    pub issues: Vec<String>,
}

#[derive(Debug)]
struct Meta {
    path: String,
    title: String,
    artist: String,
    album: String,
    genre: String,
    duration: i64,
    bitrate: Option<i64>,
    sample_rate: Option<i64>,
    track: Option<i64>,
    disc: Option<i64>,
    mtime: i64,
    file_size: i64,
}

struct DownloadedTrackState {
    mtime: i64,
    file_size: i64,
    title: String,
    artist: String,
    album_title: String,
    downloaded_album: Option<String>,
    missing: bool,
}

const COLUMNS: &str = "t.id,t.file_path,COALESCE(tt.title,t.title),ar.name,al.title,t.album_id,t.duration_ms,t.bitrate,t.sample_rate,COALESCE(t.genre,''),t.track_number,t.is_favorite,t.is_missing,t.play_count";
const JOINS: &str = " FROM tracks t JOIN artists ar ON ar.id=t.artist_id JOIN albums al ON al.id=t.album_id LEFT JOIN track_title_overrides tt ON tt.track_id=t.id ";

fn row_track(r: &Row<'_>) -> rusqlite::Result<Track> {
    Ok(Track {
        id: r.get(0)?,
        file_path: r.get(1)?,
        title: r.get(2)?,
        artist: r.get(3)?,
        album: r.get(4)?,
        album_id: r.get(5)?,
        duration_ms: r.get(6)?,
        bitrate: r.get(7)?,
        sample_rate: r.get(8)?,
        genre: r.get(9)?,
        track_number: r.get(10)?,
        is_favorite: r.get::<_, i64>(11)? != 0,
        is_missing: r.get::<_, i64>(12)? != 0,
        play_count: r.get(13)?,
    })
}

fn audio_file(p: &Path) -> bool {
    matches!(
        p.extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "mp3" | "flac" | "m4a" | "aac" | "alac" | "ogg" | "oga" | "opus" | "wav" | "aiff" | "aif"
    )
}
fn modified(p: &Path) -> (i64, i64) {
    match fs::metadata(p) {
        Ok(m) => (
            m.modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos().min(i64::MAX as u128) as i64)
                .unwrap_or(0),
            m.len() as i64,
        ),
        Err(_) => (0, 0),
    }
}
pub fn validate_audio(p: &Path) -> Result<(), String> {
    if p.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("opus"))
    {
        return Err("Formatul OPUS nu este suportat de player. Convertește fișierul în FLAC, WAV sau MP3; originalul nu este modificat.".into());
    }
    if !audio_file(p) {
        return Err("Format audio nesuportat".into());
    }
    let file = fs::File::open(p).map_err(|error| error.to_string())?;
    let mut decoder = rodio::Decoder::try_from(file)
        .map_err(|error| format!("Fișier audio invalid sau codec nesuportat: {error}"))?;
    decoder
        .next()
        .ok_or_else(|| "Fișierul audio nu conține mostre redabile".to_owned())?;
    Ok(())
}

fn analyze(p: &Path) -> Result<Meta, String> {
    validate_audio(p)?;
    let f =
        lofty::read_from_path(p).map_err(|error| format!("Metadate audio invalide: {error}"))?;
    let (mtime, file_size) = modified(p);
    let fallback = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown Track")
        .to_string();
    let mut m = Meta {
        path: p.to_string_lossy().into_owned(),
        title: fallback,
        artist: "Unknown Artist".into(),
        album: "Unknown Album".into(),
        genre: String::new(),
        duration: 0,
        bitrate: None,
        sample_rate: None,
        track: None,
        disc: None,
        mtime,
        file_size,
    };
    {
        let props = f.properties();
        m.duration = props.duration().as_millis().min(i64::MAX as u128) as i64;
        m.bitrate = props.audio_bitrate().map(i64::from);
        m.sample_rate = props.sample_rate().map(i64::from);
        if let Some(t) = f.primary_tag().or_else(|| f.first_tag()) {
            m.title = t
                .title()
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.into_owned())
                .unwrap_or(m.title);
            m.artist = t
                .artist()
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.into_owned())
                .unwrap_or(m.artist);
            m.album = t
                .album()
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.into_owned())
                .unwrap_or(m.album);
            m.genre = t.genre().map(|s| s.into_owned()).unwrap_or_default();
            m.track = t.track().map(i64::from);
            m.disc = t.disk().map(i64::from);
        }
    }
    Ok(m)
}

fn search_pattern(query: &str) -> String {
    let key = sorting::text_key(query.trim());
    format!(
        "%{}%",
        key.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}

fn missing_root_path(root: &Path) -> Result<PathBuf, String> {
    let absolute = std::path::absolute(root).map_err(|error| error.to_string())?;
    for ancestor in absolute.ancestors().skip(1) {
        if let Ok(prefix) = ancestor.canonicalize() {
            return Ok(prefix.join(
                absolute
                    .strip_prefix(ancestor)
                    .map_err(|error| error.to_string())?,
            ));
        }
    }
    Ok(absolute)
}

impl ScanStats {
    fn error(&mut self, issue: String) {
        self.errors += 1;
        if self.issues.len() < 20 {
            self.issues.push(issue);
        }
    }
}

fn downloaded_names(path: &Path) -> (String, String) {
    let title = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.trim().is_empty())
        .unwrap_or("melodie")
        .to_owned();
    let artist = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|artist| artist.to_str())
        .filter(|artist| !artist.trim().is_empty())
        .unwrap_or("Unknown Artist")
        .to_owned();
    (title, artist)
}

impl Database {
    pub fn new(path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let db = Self {
            path: Arc::new(path),
        };
        let mut conn = db.open()?;
        conn.execute_batch(r#"
            PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS artists (
                id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, biography_cache TEXT, image_path TEXT
            );
            CREATE TABLE IF NOT EXISTS albums (
                id INTEGER PRIMARY KEY, title TEXT NOT NULL, year INTEGER,
                artist_id INTEGER NOT NULL REFERENCES artists(id), cover_art_path TEXT,
                UNIQUE(title, artist_id)
            );
            CREATE TABLE IF NOT EXISTS tracks (
                id INTEGER PRIMARY KEY, file_path TEXT NOT NULL UNIQUE, title TEXT NOT NULL,
                duration_ms INTEGER NOT NULL DEFAULT 0, bitrate INTEGER, sample_rate INTEGER,
                track_number INTEGER, disc_number INTEGER, file_hash_sha256 TEXT,
                album_id INTEGER NOT NULL REFERENCES albums(id), artist_id INTEGER NOT NULL REFERENCES artists(id),
                genre TEXT, play_count INTEGER NOT NULL DEFAULT 0, last_played_at TEXT,
                is_favorite INTEGER NOT NULL DEFAULT 0, is_missing INTEGER NOT NULL DEFAULT 0,
                mtime INTEGER, file_size INTEGER
            );
            CREATE TABLE IF NOT EXISTS downloaded_track_albums (
                file_path TEXT PRIMARY KEY REFERENCES tracks(file_path) ON DELETE CASCADE,
                album_title TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS playlists (
                id INTEGER PRIMARY KEY, title TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS hidden_albums (
                album_id INTEGER PRIMARY KEY REFERENCES albums(id) ON DELETE CASCADE
            );
            CREATE TABLE IF NOT EXISTS playlist_tracks (
                playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
                track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
                position INTEGER NOT NULL, PRIMARY KEY(playlist_id, position)
            );
            CREATE TABLE IF NOT EXISTS track_title_overrides (
                track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
                title TEXT NOT NULL,
                title_key TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS library_roots (path TEXT PRIMARY KEY);
            CREATE INDEX IF NOT EXISTS tracks_title_ix ON tracks(title COLLATE NOCASE);
            CREATE INDEX IF NOT EXISTS tracks_artist_ix ON tracks(artist_id);
            CREATE INDEX IF NOT EXISTS tracks_album_ix ON tracks(album_id);
            CREATE INDEX IF NOT EXISTS tracks_favorite_ix ON tracks(is_favorite);
            CREATE INDEX IF NOT EXISTS albums_artist_ix ON albums(artist_id);
            CREATE INDEX IF NOT EXISTS tracks_file_hash_ix ON tracks(file_hash_sha256);
        "#).map_err(|e|e.to_string())?;
        let needs_migration:bool=conn.query_row("SELECT NOT EXISTS(SELECT 1 FROM pragma_table_info('tracks') WHERE name='title_key')",[],|row|row.get(0)).map_err(|error|error.to_string())?;
        let track_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if needs_migration && track_count > 0 {
            let timestamp = std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_nanos();
            let backup = db
                .path
                .with_extension(format!("before-search-migration-{timestamp}.sqlite3"));
            conn.backup("main", &backup, None).map_err(|error| {
                format!("Copia de siguranță înainte de migrare nu a putut fi creată: {error}")
            })?;
        }
        let transaction = conn.transaction().map_err(|error| error.to_string())?;
        for (table, column, original) in [
            ("tracks", "title_key", "title"),
            ("artists", "name_key", "name"),
            ("albums", "title_key", "title"),
        ] {
            let exists: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name=?2)",
                    params![table, column],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            if !exists {
                transaction.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} TEXT NOT NULL DEFAULT ''; UPDATE {table} SET {column}=aura_key({original});"))
                    .map_err(|error| error.to_string())?;
            }
            transaction.execute_batch(&format!(
                "CREATE TRIGGER IF NOT EXISTS {table}_key_insert AFTER INSERT ON {table} BEGIN UPDATE {table} SET {column}=aura_key(NEW.{original}) WHERE id=NEW.id; END;
                 CREATE TRIGGER IF NOT EXISTS {table}_key_update AFTER UPDATE OF {original} ON {table} BEGIN UPDATE {table} SET {column}=aura_key(NEW.{original}) WHERE id=NEW.id; END;"
            )).map_err(|error| error.to_string())?;
        }
        transaction.execute_batch("CREATE INDEX IF NOT EXISTS tracks_sort_title_ix ON tracks(title_key,id);
            CREATE INDEX IF NOT EXISTS tracks_sort_artist_title_ix ON tracks(artist_id,title_key,id);
            CREATE INDEX IF NOT EXISTS tracks_sort_duration_ix ON tracks(duration_ms,title_key,id);
            CREATE INDEX IF NOT EXISTS artists_name_key_ix ON artists(name_key,id);
            CREATE INDEX IF NOT EXISTS albums_title_key_ix ON albums(title_key,id);")
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(db)
    }
    pub fn open(&self) -> Result<Connection, String> {
        let c = Connection::open(self.path.as_path()).map_err(|e| e.to_string())?;
        c.create_collation("AURA_TEXT", |left, right| {
            sorting::text_key(left).cmp(&sorting::text_key(right))
        })
        .map_err(|error| error.to_string())?;
        c.create_scalar_function(
            "aura_key",
            1,
            FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
            |context| Ok(sorting::text_key(&context.get::<String>(0)?)),
        )
        .map_err(|error| error.to_string())?;
        c.busy_timeout(std::time::Duration::from_secs(10))
            .map_err(|e| e.to_string())?;
        c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=NORMAL;")
            .map_err(|e| e.to_string())?;
        Ok(c)
    }
    pub fn get(&self, id: i64) -> Result<Option<Track>, String> {
        let c = self.open()?;
        c.query_row(
            &format!("SELECT {COLUMNS}{JOINS} WHERE t.id=?1"),
            params![id],
            row_track,
        )
        .optional()
        .map_err(|e| e.to_string())
    }
    pub fn get_many(&self, ids: &[i64]) -> Result<Vec<Track>, String> {
        let connection = self.open()?;
        let mut statement = connection
            .prepare(&format!("SELECT {COLUMNS}{JOINS} WHERE t.id=?1"))
            .map_err(|error| error.to_string())?;
        let mut tracks = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(track) = statement
                .query_row(params![id], row_track)
                .optional()
                .map_err(|error| error.to_string())?
            {
                tracks.push(track);
            }
        }
        Ok(tracks)
    }

    /// Delete one explicitly selected audio file, then commit removal of its
    /// catalog entry. SQL failures and unlink failures before commit roll back
    /// the catalog transaction. No folders, covers or sidecars are removed.
    pub fn delete_audio_file(&self, id: i64, expected_path: &str) -> Result<(), String> {
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| format!("Biblioteca este ocupată: {error}"))?;
        let path: Option<String> = transaction
            .query_row(
                "SELECT file_path FROM tracks WHERE id=?1",
                params![id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let path = path.ok_or("Piesa nu mai există în bibliotecă")?;
        if path != expected_path {
            return Err("Fișierul piesei s-a schimbat; ștergerea a fost anulată".into());
        }
        let file = Path::new(&path);
        if !audio_file(file) {
            return Err("Fișierul selectat nu este un fișier audio acceptat".into());
        }
        match fs::symlink_metadata(file) {
            Ok(metadata) if !metadata.file_type().is_file() => {
                return Err("Ștergerea este permisă numai pentru un fișier audio obișnuit".into())
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(format!("Fișier inaccesibil: {error}"))
            }
            _ => {}
        }
        transaction
            .execute("DELETE FROM tracks WHERE id=?1", params![id])
            .map_err(|error| error.to_string())?;
        match fs::remove_file(file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Nu am putut șterge fișierul audio: {error}")),
        }
        transaction.commit().map_err(|error| {
            format!("Fișierul a fost șters, dar biblioteca nu a putut fi actualizată: {error}")
        })
    }
    #[cfg(test)]
    pub fn page(
        &self,
        offset: i64,
        limit: i64,
        query: &str,
        favorites: bool,
        album_id: Option<i64>,
    ) -> Result<Page, String> {
        self.page_sorted(
            offset,
            limit,
            query,
            favorites,
            album_id,
            TrackSort::default(),
        )
    }
    pub fn page_sorted(
        &self,
        offset: i64,
        limit: i64,
        query: &str,
        favorites: bool,
        album_id: Option<i64>,
        sort: TrackSort,
    ) -> Result<Page, String> {
        let c = self.open()?;
        let q = search_pattern(query);
        let filter=" WHERE t.is_missing=0 AND (?1=0 OR t.is_favorite=1) AND (?2 IS NULL OR t.album_id=?2) AND (COALESCE(tt.title_key,t.title_key) LIKE ?3 ESCAPE '\\' OR ar.name_key LIKE ?3 ESCAPE '\\' OR al.title_key LIKE ?3 ESCAPE '\\') ";
        let total: i64 = c
            .query_row(
                &format!("SELECT COUNT(*){JOINS}{filter}"),
                params![favorites as i32, album_id, q],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let mut st = c
            .prepare(&format!(
                "SELECT {COLUMNS}{JOINS}{filter} ORDER BY {} LIMIT ?4 OFFSET ?5",
                sort.sql()
            ))
            .map_err(|e| e.to_string())?;
        let tracks = st
            .query_map(
                params![
                    favorites as i32,
                    album_id,
                    q,
                    limit.clamp(1, 300),
                    offset.max(0)
                ],
                row_track,
            )
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        Ok(Page { total, tracks })
    }
    pub fn albums(&self, query: &str) -> Result<Vec<Album>, String> {
        let c = self.open()?;
        let sql="SELECT al.id,al.title,ar.name,COUNT(t.id),SUM(t.duration_ms),MIN(t.id),(SELECT cover.file_path FROM tracks cover WHERE cover.album_id=al.id AND cover.is_missing=0 ORDER BY cover.id LIMIT 1) FROM albums al JOIN artists ar ON ar.id=al.artist_id JOIN tracks t ON t.album_id=al.id AND t.is_missing=0 WHERE NOT EXISTS(SELECT 1 FROM hidden_albums ha WHERE ha.album_id=al.id) AND (al.title_key LIKE ?1 ESCAPE '\\' OR ar.name_key LIKE ?1 ESCAPE '\\') GROUP BY al.id ORDER BY al.title_key,al.id";
        let mut st = c.prepare(sql).map_err(|e| e.to_string())?;
        let albums = st
            .query_map(params![search_pattern(query)], |r| {
                Ok(Album {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    artist: r.get(2)?,
                    track_count: r.get(3)?,
                    duration_ms: r.get(4)?,
                    representative_track: r.get(5)?,
                    representative_path: r.get(6)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        Ok(albums)
    }
    pub fn playlists(&self) -> Result<Vec<Playlist>, String> {
        let connection = self.open()?;
        let mut statement = connection
            .prepare("SELECT p.id,p.title,COUNT(t.id) FROM playlists p LEFT JOIN playlist_tracks pt ON pt.playlist_id=p.id LEFT JOIN tracks t ON t.id=pt.track_id AND t.is_missing=0 GROUP BY p.id ORDER BY p.title COLLATE NOCASE,p.id")
            .map_err(|error| error.to_string())?;
        let playlists = statement
            .query_map([], |row| {
                Ok(Playlist {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    track_count: row.get(2)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        Ok(playlists)
    }
    pub fn create_playlist(&self, title: &str) -> Result<i64, String> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 120 || title.chars().any(char::is_control) {
            return Err("Numele playlistului trebuie să aibă între 1 și 120 de caractere.".into());
        }
        let connection = self.open()?;
        let already_exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM playlists WHERE title=?1 COLLATE NOCASE)",
                params![title],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if already_exists {
            return Err("Există deja un playlist cu acest nume.".into());
        }
        connection
            .execute("INSERT INTO playlists(title) VALUES(?1)", params![title])
            .map_err(|error| error.to_string())?;
        Ok(connection.last_insert_rowid())
    }
    pub fn create_or_get_playlist(&self, title: &str) -> Result<i64, String> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 120 || title.chars().any(char::is_control) {
            return Err("Numele playlistului trebuie să aibă între 1 și 120 de caractere.".into());
        }
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let existing: Option<i64> = transaction
            .query_row(
                "SELECT id FROM playlists WHERE title=?1 COLLATE NOCASE ORDER BY id LIMIT 1",
                params![title],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let id = if let Some(id) = existing {
            id
        } else {
            transaction
                .execute("INSERT INTO playlists(title) VALUES(?1)", params![title])
                .map_err(|error| error.to_string())?;
            transaction.last_insert_rowid()
        };
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(id)
    }
    pub fn rename_playlist(&self, playlist_id: i64, title: &str) -> Result<String, String> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 120 || title.chars().any(char::is_control) {
            return Err("Numele playlistului trebuie să aibă între 1 și 120 de caractere.".into());
        }
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM playlists WHERE id=?1)",
                params![playlist_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if !exists {
            return Err("Playlistul nu mai există.".into());
        }
        let duplicate: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM playlists WHERE title=?1 COLLATE NOCASE AND id<>?2)",
                params![title, playlist_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if duplicate {
            return Err("Există deja un playlist cu acest nume.".into());
        }
        transaction
            .execute(
                "UPDATE playlists SET title=?1 WHERE id=?2",
                params![title, playlist_id],
            )
            .map_err(|error| format!("Nu am putut redenumi playlistul: {error}"))?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(title.to_owned())
    }
    pub fn delete_playlist(&self, playlist_id: i64) -> Result<(), String> {
        let connection = self.open()?;
        connection
            .execute("DELETE FROM playlists WHERE id=?1", params![playlist_id])
            .map_err(|error| format!("Nu am putut șterge playlistul: {error}"))?;
        Ok(())
    }
    pub fn hide_album(&self, album_id: i64) -> Result<(), String> {
        self.open()?
            .execute(
                "INSERT OR IGNORE INTO hidden_albums(album_id) VALUES(?1)",
                params![album_id],
            )
            .map_err(|error| format!("Nu am putut elimina albumul din bibliotecă: {error}"))?;
        Ok(())
    }
    pub fn add_track_to_playlist(&self, playlist_id: i64, track_id: i64) -> Result<(), String> {
        let connection = self.open()?;
        let playlist_exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM playlists WHERE id=?1)",
                params![playlist_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if !playlist_exists {
            return Err("Playlistul nu mai există.".into());
        }
        let track_exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM tracks WHERE id=?1 AND is_missing=0)",
                params![track_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if !track_exists {
            return Err("Melodia nu mai este disponibilă în bibliotecă.".into());
        }
        let already_added: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM playlist_tracks WHERE playlist_id=?1 AND track_id=?2)",
                params![playlist_id, track_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if already_added {
            return Ok(());
        }
        let position: i64 = connection
            .query_row(
                "SELECT COALESCE(MAX(position)+1,0) FROM playlist_tracks WHERE playlist_id=?1",
                params![playlist_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "INSERT INTO playlist_tracks(playlist_id,track_id,position) VALUES(?1,?2,?3)",
                params![playlist_id, track_id, position],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
    pub fn playlist_page_sorted(
        &self,
        playlist_id: i64,
        offset: i64,
        limit: i64,
        query: &str,
        sort: TrackSort,
    ) -> Result<Page, String> {
        let connection = self.open()?;
        let pattern = search_pattern(query);
        let filter = " FROM playlist_tracks pt JOIN tracks t ON t.id=pt.track_id JOIN artists ar ON ar.id=t.artist_id JOIN albums al ON al.id=t.album_id LEFT JOIN track_title_overrides tt ON tt.track_id=t.id WHERE pt.playlist_id=?1 AND t.is_missing=0 AND (COALESCE(tt.title_key,t.title_key) LIKE ?2 ESCAPE '\\' OR ar.name_key LIKE ?2 ESCAPE '\\' OR al.title_key LIKE ?2 ESCAPE '\\')";
        let total: i64 = connection
            .query_row(
                &format!("SELECT COUNT(*){filter}"),
                params![playlist_id, pattern],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let order = if sort.column == sorting::SortColumn::Library {
            "pt.position"
        } else {
            sort.sql()
        };
        let mut statement = connection
            .prepare(&format!(
                "SELECT {COLUMNS}{filter} ORDER BY {order} LIMIT ?3 OFFSET ?4"
            ))
            .map_err(|error| error.to_string())?;
        let tracks = statement
            .query_map(
                params![playlist_id, pattern, limit.clamp(1, 300), offset.max(0)],
                row_track,
            )
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        Ok(Page { total, tracks })
    }
    pub fn playlist_order_sorted(
        &self,
        playlist_id: i64,
        query: &str,
        sort: TrackSort,
    ) -> Result<Vec<i64>, String> {
        let connection = self.open()?;
        let order = if sort.column == sorting::SortColumn::Library {
            "pt.position"
        } else {
            sort.sql()
        };
        let mut statement = connection
            .prepare(&format!("SELECT t.id{JOINS} JOIN playlist_tracks pt ON pt.track_id=t.id WHERE pt.playlist_id=?1 AND t.is_missing=0 AND (COALESCE(tt.title_key,t.title_key) LIKE ?2 ESCAPE '\\' OR ar.name_key LIKE ?2 ESCAPE '\\' OR al.title_key LIKE ?2 ESCAPE '\\') ORDER BY {order}"))
            .map_err(|error| error.to_string())?;
        let ids = statement
            .query_map(params![playlist_id, search_pattern(query)], |row| {
                row.get(0)
            })
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        Ok(ids)
    }
    pub fn rename_track(&self, track_id: i64, title: &str) -> Result<String, String> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 200 || title.chars().any(char::is_control) {
            return Err("Titlul trebuie să aibă între 1 și 200 de caractere.".into());
        }
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM tracks WHERE id=?1)",
                params![track_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if !exists {
            return Err("Melodia nu mai există în bibliotecă.".into());
        }
        transaction
            .execute("INSERT INTO track_title_overrides(track_id,title,title_key) VALUES(?1,?2,?3) ON CONFLICT(track_id) DO UPDATE SET title=excluded.title,title_key=excluded.title_key", params![track_id, title, sorting::text_key(title)])
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(title.to_owned())
    }
    pub fn rename_album(&self, album_id: i64, title: &str) -> Result<String, String> {
        let title = title.trim();
        if title.is_empty() {
            return Err("Numele albumului nu poate fi gol.".into());
        }
        if title.chars().count() > 120 || title.chars().any(char::is_control) {
            return Err("Numele albumului trebuie să aibă cel mult 120 de caractere.".into());
        }
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let artist_id: i64 = transaction
            .query_row(
                "SELECT artist_id FROM albums WHERE id=?1",
                params![album_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or("Albumul nu mai există.")?;
        let duplicate: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM albums WHERE title=?1 AND artist_id=?2 AND id<>?3)",
                params![title, artist_id, album_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if duplicate {
            return Err("Există deja un album cu acest nume pentru artistul albumului.".into());
        }
        transaction
            .execute(
                "UPDATE albums SET title=?1 WHERE id=?2",
                params![title, album_id],
            )
            .map_err(|e| format!("Nu am putut redenumi albumul: {e}"))?;
        transaction.execute(
            "UPDATE downloaded_track_albums SET album_title=?1 WHERE file_path IN (SELECT file_path FROM tracks WHERE album_id=?2)",
            params![title, album_id],
        ).map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(title.to_owned())
    }
    pub fn favorites(&self, id: i64) -> Result<bool, String> {
        let c = self.open()?;
        c.execute(
            "UPDATE tracks SET is_favorite=1-is_favorite WHERE id=?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        let b: i64 = c
            .query_row(
                "SELECT is_favorite FROM tracks WHERE id=?1",
                params![id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(b != 0)
    }
    #[cfg(test)]
    pub fn order(&self, album_id: Option<i64>, favorites: bool) -> Result<Vec<i64>, String> {
        self.order_sorted(album_id, favorites, "", TrackSort::default())
    }
    pub fn order_sorted(
        &self,
        album_id: Option<i64>,
        favorites: bool,
        query: &str,
        sort: TrackSort,
    ) -> Result<Vec<i64>, String> {
        let c = self.open()?;
        let mut st=c.prepare(&format!("SELECT t.id{JOINS} WHERE t.is_missing=0 AND (?1 IS NULL OR t.album_id=?1) AND (?2=0 OR t.is_favorite=1) AND (t.title_key LIKE ?3 ESCAPE '\\' OR ar.name_key LIKE ?3 ESCAPE '\\' OR al.title_key LIKE ?3 ESCAPE '\\') ORDER BY {}",sort.sql())).map_err(|e|e.to_string())?;
        let ids = st
            .query_map(
                params![album_id, favorites as i32, search_pattern(query)],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        Ok(ids)
    }
    pub fn note_play(&self, id: i64) {
        if let Ok(c) = self.open() {
            let _=c.execute("UPDATE tracks SET play_count=play_count+1,last_played_at=datetime('now'),is_missing=0 WHERE id=?1",params![id]);
        }
    }
    pub fn missing(&self, id: i64) {
        if let Ok(c) = self.open() {
            let _ = c.execute("UPDATE tracks SET is_missing=1 WHERE id=?1", params![id]);
        }
    }
    pub fn roots(&self) -> Result<Vec<String>, String> {
        let c = self.open()?;
        let mut st = c
            .prepare("SELECT path FROM library_roots ORDER BY path")
            .map_err(|e| e.to_string())?;
        let roots = st
            .query_map([], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        Ok(roots)
    }
    pub fn scan(&self, root: &str) -> Result<ScanStats, String> {
        let root = Path::new(root).canonicalize().map_err(|e| e.to_string())?;
        if !root.is_dir() {
            return Err("Selectați un director valid".into());
        }
        let root_str = root.to_string_lossy().into_owned();
        let mut c = self.open()?;
        let mut existing = HashMap::<String, (i64, i64, bool)>::new();
        {
            let mut stmt=c.prepare("SELECT file_path,COALESCE(mtime,0),COALESCE(file_size,0),is_missing FROM tracks").map_err(|e|e.to_string())?;
            let iter = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, bool>(3)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            for item in iter {
                let (p, t, s, missing) = item.map_err(|e| e.to_string())?;
                if Path::new(&p).starts_with(&root) {
                    existing.insert(p, (t, s, missing));
                }
            }
        }
        let mut stats = ScanStats {
            discovered: 0,
            updated: 0,
            missing: 0,
            errors: 0,
            issues: vec![],
        };
        let mut walk_complete = true;
        let mut paths = Vec::new();
        for entry in WalkDir::new(&root).follow_links(false) {
            match entry {
                Ok(entry) if entry.file_type().is_file() && audio_file(entry.path()) => {
                    paths.push(entry.into_path())
                }
                Ok(_) => {}
                Err(error) => {
                    walk_complete = false;
                    stats.error(error.to_string());
                }
            }
        }
        stats.discovered = paths.len();
        let present: HashSet<String> = paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let changed: Vec<&PathBuf> = paths
            .iter()
            .filter(|p| {
                let key = p.to_string_lossy();
                let info = modified(p);
                existing.get(key.as_ref()).copied() != Some((info.0, info.1, false))
            })
            .collect();
        let results: Vec<_> = changed.par_iter().map(|p| (*p, analyze(p))).collect();
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        for (path, result) in &results {
            // A deletion may have completed while metadata was analyzed.
            // Recheck under the writer lock so stale scans cannot reinsert it.
            if !path.is_file() {
                continue;
            }
            let m = match result {
                Ok(metadata) => metadata,
                Err(error) => {
                    stats.error(format!("{}: {error}", path.display()));
                    tx.execute(
                        "UPDATE tracks SET is_missing=1 WHERE file_path=?1",
                        params![path.to_string_lossy().as_ref()],
                    )
                    .map_err(|e| e.to_string())?;
                    continue;
                }
            };
            let result = (|| -> rusqlite::Result<()> {
                let downloaded_album: Option<String> = tx
                    .query_row(
                        "SELECT album_title FROM downloaded_track_albums WHERE file_path=?1",
                        params![m.path],
                        |row| row.get(0),
                    )
                    .optional()?;
                let album_title = downloaded_album.as_deref().unwrap_or(&m.album);
                let album_artist_name = if downloaded_album.is_some() {
                    "Descarcate"
                } else {
                    m.artist.as_str()
                };
                tx.execute(
                    "INSERT OR IGNORE INTO artists(name) VALUES(?1)",
                    params![m.artist],
                )?;
                let artist_id: i64 = tx.query_row(
                    "SELECT id FROM artists WHERE name=?1",
                    params![m.artist],
                    |r| r.get(0),
                )?;
                tx.execute(
                    "INSERT OR IGNORE INTO artists(name) VALUES(?1)",
                    params![album_artist_name],
                )?;
                let album_artist_id: i64 = tx.query_row(
                    "SELECT id FROM artists WHERE name=?1",
                    params![album_artist_name],
                    |r| r.get(0),
                )?;
                tx.execute(
                    "INSERT OR IGNORE INTO albums(title,artist_id) VALUES(?1,?2)",
                    params![album_title, album_artist_id],
                )?;
                let album_id: i64 = tx.query_row(
                    "SELECT id FROM albums WHERE title=?1 AND artist_id=?2",
                    params![album_title, album_artist_id],
                    |r| r.get(0),
                )?;
                tx.execute(r#"INSERT INTO tracks(file_path,title,duration_ms,bitrate,sample_rate,track_number,disc_number,album_id,artist_id,genre,mtime,file_size,is_missing)
                    VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,0)
                    ON CONFLICT(file_path) DO UPDATE SET title=excluded.title,duration_ms=excluded.duration_ms,bitrate=excluded.bitrate,sample_rate=excluded.sample_rate,
                    track_number=excluded.track_number,disc_number=excluded.disc_number,album_id=excluded.album_id,artist_id=excluded.artist_id,genre=excluded.genre,
                    mtime=excluded.mtime,file_size=excluded.file_size,is_missing=0"#,
                    params![m.path,m.title,m.duration,m.bitrate,m.sample_rate,m.track,m.disc,album_id,artist_id,m.genre,m.mtime,m.file_size])?;
                Ok(())
            })();
            match result {
                Ok(()) => stats.updated += 1,
                Err(error) => stats.error(format!("{}: {error}", path.display())),
            }
        }
        if walk_complete {
            for p in existing.keys().filter(|p| !present.contains(*p)) {
                stats.missing += tx
                    .execute(
                        "UPDATE tracks SET is_missing=1 WHERE file_path=?1 AND is_missing=0",
                        params![p],
                    )
                    .map_err(|e| e.to_string())?;
            }
        }
        tx.execute(
            "INSERT OR IGNORE INTO library_roots(path) VALUES(?1)",
            params![root_str],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(stats)
    }
    #[cfg(test)]
    pub fn import_downloaded(&self, file_path: &Path) -> Result<(), String> {
        self.import_downloaded_to_album(file_path, "Descarcate")
    }
    pub fn import_downloaded_to_album(
        &self,
        file_path: &Path,
        download_album: &str,
    ) -> Result<(), String> {
        self.import_downloaded_to_album_inner(file_path, download_album, None)
    }
    pub fn import_downloaded_to_album_and_playlist(
        &self,
        file_path: &Path,
        download_album: &str,
        playlist_id: i64,
    ) -> Result<(), String> {
        self.import_downloaded_to_album_inner(file_path, download_album, Some(playlist_id))
    }
    fn import_downloaded_to_album_inner(
        &self,
        file_path: &Path,
        download_album: &str,
        playlist_id: Option<i64>,
    ) -> Result<(), String> {
        let download_album = download_album.trim();
        if download_album.is_empty()
            || download_album.chars().count() > 120
            || download_album.chars().any(char::is_control)
        {
            return Err("Numele playlistului trebuie să aibă între 1 și 120 de caractere.".into());
        }
        let file_path = file_path.canonicalize().map_err(|e| e.to_string())?;
        if !audio_file(&file_path) {
            return Err("Formatul fișierului nu este acceptat".into());
        }
        let mut metadata = analyze(&file_path)?;
        (metadata.title, metadata.artist) = downloaded_names(&file_path);
        metadata.album = download_album.to_owned();
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        if !file_path.is_file() {
            return Err("Fișierul audio a dispărut înainte de import".into());
        }
        if let Some(playlist_id) = playlist_id {
            let playlist_exists: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM playlists WHERE id=?1)",
                    params![playlist_id],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            if !playlist_exists {
                return Err("Playlistul nu mai există.".into());
            }
        }
        transaction
            .execute(
                "INSERT OR IGNORE INTO artists(name) VALUES('Descarcate')",
                [],
            )
            .map_err(|e| e.to_string())?;
        let album_artist_id: i64 = transaction
            .query_row(
                "SELECT id FROM artists WHERE name='Descarcate'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO artists(name) VALUES(?1)",
                params![metadata.artist],
            )
            .map_err(|e| e.to_string())?;
        let artist_id: i64 = transaction
            .query_row(
                "SELECT id FROM artists WHERE name=?1",
                params![metadata.artist],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO albums(title,artist_id) VALUES(?1,?2)",
                params![download_album, album_artist_id],
            )
            .map_err(|e| e.to_string())?;
        let album_id: i64 = transaction
            .query_row(
                "SELECT id FROM albums WHERE title=?1 AND artist_id=?2",
                params![download_album, album_artist_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        transaction.execute(
            r#"INSERT INTO tracks(file_path,title,duration_ms,bitrate,sample_rate,track_number,disc_number,album_id,artist_id,genre,mtime,file_size,is_missing)
                VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,0)
                ON CONFLICT(file_path) DO UPDATE SET title=excluded.title,duration_ms=excluded.duration_ms,
                bitrate=excluded.bitrate,sample_rate=excluded.sample_rate,track_number=excluded.track_number,
                disc_number=excluded.disc_number,album_id=excluded.album_id,artist_id=excluded.artist_id,
                genre=excluded.genre,mtime=excluded.mtime,file_size=excluded.file_size,is_missing=0"#,
            params![
                metadata.path, metadata.title, metadata.duration, metadata.bitrate,
                metadata.sample_rate, metadata.track, metadata.disc, album_id, artist_id,
                metadata.genre, metadata.mtime, metadata.file_size,
            ],
        ).map_err(|e| e.to_string())?;
        transaction.execute(
            "INSERT INTO downloaded_track_albums(file_path,album_title) VALUES(?1,?2) ON CONFLICT(file_path) DO UPDATE SET album_title=excluded.album_title",
            params![metadata.path, download_album],
        ).map_err(|e| e.to_string())?;
        if let Some(playlist_id) = playlist_id {
            let track_id: i64 = transaction
                .query_row(
                    "SELECT id FROM tracks WHERE file_path=?1 AND is_missing=0",
                    params![metadata.path],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            let already_added: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM playlist_tracks WHERE playlist_id=?1 AND track_id=?2)",
                    params![playlist_id, track_id],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            if !already_added {
                let position: i64 = transaction
                    .query_row(
                        "SELECT COALESCE(MAX(position)+1,0) FROM playlist_tracks WHERE playlist_id=?1",
                        params![playlist_id],
                        |row| row.get(0),
                    )
                    .map_err(|error| error.to_string())?;
                transaction
                    .execute(
                        "INSERT INTO playlist_tracks(playlist_id,track_id,position) VALUES(?1,?2,?3)",
                        params![playlist_id, track_id, position],
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
        transaction.commit().map_err(|e| e.to_string())
    }
    pub fn scan_downloaded_music(&self, root: &Path) -> Result<ScanStats, String> {
        if let Err(error) = fs::metadata(root) {
            if error.kind() != std::io::ErrorKind::NotFound {
                return Err(format!("Folder Music inaccesibil: {error}"));
            }
            let missing_root = missing_root_path(root)?;
            let mut connection = self.open()?;
            let transaction = connection
                .transaction()
                .map_err(|error| error.to_string())?;
            let paths: Vec<(i64, String)> = {
                let mut statement = transaction
                    .prepare("SELECT id,file_path FROM tracks WHERE is_missing=0")
                    .map_err(|error| error.to_string())?;
                let rows = statement
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                    .map_err(|error| error.to_string())?;
                rows.collect::<rusqlite::Result<_>>()
                    .map_err(|error| error.to_string())?
            };
            let mut missing = 0;
            for (id, path) in paths {
                if Path::new(&path).starts_with(&missing_root) {
                    missing += transaction
                        .execute("UPDATE tracks SET is_missing=1 WHERE id=?1", params![id])
                        .map_err(|error| error.to_string())?;
                }
            }
            transaction.commit().map_err(|error| error.to_string())?;
            return Ok(ScanStats {
                discovered: 0,
                updated: 0,
                missing,
                errors: 0,
                issues: if missing > 0 {
                    vec![format!(
                        "Folderul Music este indisponibil: {}. Favoritele sunt păstrate.",
                        root.display()
                    )]
                } else {
                    vec![]
                },
            });
        }
        let root = root.canonicalize().map_err(|error| error.to_string())?;
        if !root.is_dir() {
            return Err("Folderul Music nu este un director valid".into());
        }
        let mut existing = HashMap::new();
        {
            let connection = self.open()?;
            let mut statement = connection
                .prepare(
                    "SELECT t.file_path, COALESCE(t.mtime,0), COALESCE(t.file_size,0),
                        t.title, ar.name, al.title,
                        COALESCE(dta.album_title, CASE WHEN al.title='Descarcate' AND album_artist.name='Descarcate' THEN 'Descarcate' END),
                        t.is_missing
                    FROM tracks t JOIN artists ar ON ar.id=t.artist_id
                    JOIN albums al ON al.id=t.album_id
                    JOIN artists album_artist ON album_artist.id=al.artist_id
                    LEFT JOIN downloaded_track_albums dta ON dta.file_path=t.file_path",
                )
                .map_err(|error| error.to_string())?;
            let paths = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        DownloadedTrackState {
                            mtime: row.get(1)?,
                            file_size: row.get(2)?,
                            title: row.get(3)?,
                            artist: row.get(4)?,
                            album_title: row.get(5)?,
                            downloaded_album: row.get(6)?,
                            missing: row.get(7)?,
                        },
                    ))
                })
                .map_err(|error| error.to_string())?;
            for path in paths {
                let (path, state) = path.map_err(|error| error.to_string())?;
                if Path::new(&path).starts_with(&root) {
                    existing.insert(path, state);
                }
            }
        }
        let mut present = HashSet::new();
        let mut discovered = 0;
        let mut updated = 0;
        let mut errors = 0;
        let mut walk_complete = true;
        let mut issues = Vec::new();
        for entry in WalkDir::new(&root).follow_links(false) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    errors += 1;
                    if issues.len() < 20 {
                        issues.push(error.to_string());
                    }
                    walk_complete = false;
                    continue;
                }
            };
            if !entry.file_type().is_file() || !audio_file(entry.path()) {
                continue;
            }
            discovered += 1;
            let path = match entry.path().canonicalize() {
                Ok(path) => path,
                Err(error) => {
                    errors += 1;
                    if issues.len() < 20 {
                        issues.push(error.to_string());
                    }
                    walk_complete = false;
                    continue;
                }
            };
            let key = path.to_string_lossy().into_owned();
            present.insert(key.clone());
            let (mtime, file_size) = modified(&path);
            let (title, artist) = downloaded_names(&path);
            if !path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("opus"))
                && existing.get(&key).is_some_and(|state| {
                    state.mtime == mtime
                        && state.file_size == file_size
                        && state.title == title
                        && state.artist == artist
                        && state.downloaded_album.as_deref() == Some(state.album_title.as_str())
                        && !state.missing
                })
            {
                continue;
            }
            let download_album = existing
                .get(&key)
                .and_then(|state| state.downloaded_album.as_deref())
                .unwrap_or("Descarcate");
            match self.import_downloaded_to_album(&path, download_album) {
                Ok(()) => updated += 1,
                Err(error) => {
                    errors += 1;
                    if issues.len() < 20 {
                        issues.push(format!("{}: {error}", path.display()));
                    }
                    let connection = self.open()?;
                    connection
                        .execute(
                            "UPDATE tracks SET is_missing=1 WHERE file_path=?1",
                            params![key],
                        )
                        .map_err(|error| error.to_string())?;
                }
            }
        }
        let mut missing = 0;
        // An unreadable directory must not hide tracks that may still exist there.
        if walk_complete {
            let mut connection = self.open()?;
            let transaction = connection
                .transaction()
                .map_err(|error| error.to_string())?;
            for (path, state) in &existing {
                if !state.missing && !present.contains(path) {
                    missing += transaction
                        .execute(
                            "UPDATE tracks SET is_missing=1 WHERE file_path=?1 AND is_missing=0",
                            params![path],
                        )
                        .map_err(|error| error.to_string())?;
                }
            }
            transaction.commit().map_err(|error| error.to_string())?;
        }
        Ok(ScanStats {
            discovered,
            updated,
            missing,
            errors,
            issues,
        })
    }
    pub fn lyrics(&self, id: i64) -> Result<String, String> {
        let t = self.get(id)?.ok_or("Piesă inexistentă")?;
        let lrc = Path::new(&t.file_path).with_extension("lrc");
        let read = |path: PathBuf| -> std::io::Result<String> {
            use std::io::Read;
            let mut bytes = Vec::new();
            fs::File::open(path)?
                .take(1_048_577)
                .read_to_end(&mut bytes)?;
            if bytes.len() > 1_048_576 {
                return Err(std::io::Error::other("Versurile depășesc 1 MiB"));
            }
            String::from_utf8(bytes).map_err(std::io::Error::other)
        };
        read(lrc)
            .or_else(|_| read(Path::new(&t.file_path).with_extension("txt")))
            .map_err(|_| "Nu există versuri locale (.lrc sau .txt)".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{downloaded_names, Database};
    use crate::test_support::{write_test_mp3, TestLibrary};
    use lofty::{
        config::WriteOptions,
        tag::{Accessor, Tag, TagExt, TagType},
    };
    use std::collections::HashSet;
    use std::path::Path;

    #[test]
    fn downloaded_playlist_tracks_are_in_the_downloads_album_and_playlist() {
        let library = TestLibrary::new("downloaded-playlist");
        let path = library.mp3("Artist", "First Track");
        let playlist_id = library.db.create_or_get_playlist("Road Mix").unwrap();
        assert_eq!(
            library.db.create_or_get_playlist(" road mix ").unwrap(),
            playlist_id
        );

        library
            .db
            .import_downloaded_to_album_and_playlist(&path, "Descarcate", playlist_id)
            .unwrap();
        // Re-importing the same downloaded file must not duplicate the playlist entry.
        library
            .db
            .import_downloaded_to_album_and_playlist(&path, "Descarcate", playlist_id)
            .unwrap();

        let track = library
            .db
            .page(0, 10, "", false, None)
            .unwrap()
            .tracks
            .remove(0);
        assert_eq!(track.album, "Descarcate");
        assert_eq!(library.db.albums("").unwrap()[0].title, "Descarcate");
        let playlist_page = library
            .db
            .playlist_page_sorted(playlist_id, 0, 10, "", crate::sorting::TrackSort::default())
            .unwrap();
        assert_eq!(playlist_page.total, 1);
        assert_eq!(playlist_page.tracks[0].id, track.id);
    }

    #[test]
    fn playlists_can_be_renamed_without_colliding_with_another_playlist() {
        let library = TestLibrary::new("rename-playlist");
        let playlist_id = library.db.create_playlist("Road Mix").unwrap();
        library.db.create_playlist("Study Mix").unwrap();

        assert_eq!(
            library
                .db
                .rename_playlist(playlist_id, "  Evening Mix  ")
                .unwrap(),
            "Evening Mix"
        );
        assert!(library
            .db
            .rename_playlist(playlist_id, "study mix")
            .is_err());
        assert!(library.db.rename_playlist(playlist_id, " ").is_err());
        assert_eq!(library.db.playlists().unwrap()[0].title, "Evening Mix");
    }

    #[test]
    fn deleting_a_playlist_preserves_music_and_hiding_an_album_keeps_tracks_available() {
        let library = TestLibrary::new("collection-deletion");
        library.mp3("Artist", "Song");
        library.db.scan_downloaded_music(&library.music).unwrap();
        let track = library
            .db
            .page(0, 10, "", false, None)
            .unwrap()
            .tracks
            .remove(0);
        let playlist_id = library.db.create_playlist("Keep Music").unwrap();
        library
            .db
            .add_track_to_playlist(playlist_id, track.id)
            .unwrap();

        library.db.delete_playlist(playlist_id).unwrap();
        assert!(library.db.playlists().unwrap().is_empty());
        assert!(library.db.get(track.id).unwrap().is_some());

        assert_eq!(library.db.albums("").unwrap().len(), 1);
        library.db.hide_album(track.album_id).unwrap();
        assert!(library.db.albums("").unwrap().is_empty());
        assert_eq!(library.db.page(0, 10, "", false, None).unwrap().total, 1);
    }

    #[test]
    fn playlists_keep_tracks_in_the_library_and_track_renames_survive_rescans() {
        let library = TestLibrary::new("user-playlists");
        let path = library.mp3("Artist", "Original Title");
        library.db.scan_downloaded_music(&library.music).unwrap();
        let track = library
            .db
            .page(0, 10, "", false, None)
            .unwrap()
            .tracks
            .remove(0);
        let playlist = library.db.create_playlist("Road Mix").unwrap();
        assert!(library.db.create_playlist(" road mix ").is_err());
        library
            .db
            .add_track_to_playlist(playlist, track.id)
            .unwrap();
        library
            .db
            .add_track_to_playlist(playlist, track.id)
            .unwrap();
        assert_eq!(library.db.playlists().unwrap()[0].track_count, 1);
        assert_eq!(
            library
                .db
                .playlist_page_sorted(playlist, 0, 10, "", crate::sorting::TrackSort::default())
                .unwrap()
                .tracks[0]
                .id,
            track.id
        );
        assert_eq!(
            library
                .db
                .page(0, 10, "", false, Some(track.album_id))
                .unwrap()
                .total,
            1
        );

        library.db.rename_track(track.id, "Renamed Title").unwrap();
        assert_eq!(
            library.db.get(track.id).unwrap().unwrap().title,
            "Renamed Title"
        );
        assert_eq!(
            library
                .db
                .page(0, 10, "Renamed", false, None)
                .unwrap()
                .total,
            1
        );
        assert_eq!(
            library
                .db
                .page(0, 10, "Original", false, None)
                .unwrap()
                .total,
            0
        );
        library.db.scan_downloaded_music(&library.music).unwrap();
        assert_eq!(
            library.db.get(track.id).unwrap().unwrap().title,
            "Renamed Title"
        );
        assert!(path.is_file());
    }

    #[test]
    fn sorting_is_global_reversible_and_shared_with_filtered_playback_order() {
        use crate::sorting::{SortColumn, TrackSort};
        let library = TestLibrary::new("sorted-library");
        for (artist, title) in [
            ("Șerban", "Zulu"),
            ("ana", "alpha"),
            ("Zed", "Ăna"),
            ("beta", "Șarpe"),
        ] {
            library.mp3(artist, title);
        }
        library.db.scan_downloaded_music(&library.music).unwrap();
        for track in library.db.page(0, 20, "", false, None).unwrap().tracks {
            let duration = match track.title.as_str() {
                "Zulu" => 3500,
                "alpha" => 2000,
                "Ăna" => 1000,
                _ => 4000,
            };
            library
                .db
                .open()
                .unwrap()
                .execute(
                    "UPDATE tracks SET duration_ms=?1 WHERE id=?2",
                    rusqlite::params![duration, track.id],
                )
                .unwrap();
            if track.title == "Zulu" || track.title == "Șarpe" {
                library.db.favorites(track.id).unwrap();
            }
        }
        for (column, descending, expected) in [
            (
                SortColumn::Title,
                false,
                vec!["alpha", "Ăna", "Șarpe", "Zulu"],
            ),
            (
                SortColumn::Title,
                true,
                vec!["Zulu", "Șarpe", "Ăna", "alpha"],
            ),
            (
                SortColumn::Artist,
                false,
                vec!["alpha", "Șarpe", "Zulu", "Ăna"],
            ),
            (
                SortColumn::Artist,
                true,
                vec!["Ăna", "Zulu", "Șarpe", "alpha"],
            ),
            (
                SortColumn::Duration,
                true,
                vec!["Șarpe", "Zulu", "alpha", "Ăna"],
            ),
            (
                SortColumn::Duration,
                false,
                vec!["Ăna", "alpha", "Zulu", "Șarpe"],
            ),
        ] {
            let sort = TrackSort { column, descending };
            let mut tracks = library
                .db
                .page_sorted(0, 2, "", false, None, sort)
                .unwrap()
                .tracks;
            tracks.extend(
                library
                    .db
                    .page_sorted(2, 2, "", false, None, sort)
                    .unwrap()
                    .tracks,
            );
            assert_eq!(
                tracks
                    .iter()
                    .map(|track| track.title.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(
                tracks.iter().map(|track| track.id).collect::<Vec<_>>(),
                library.db.order_sorted(None, false, "", sort).unwrap()
            );
            let favorites = library
                .db
                .page_sorted(0, 20, "Ș", true, None, sort)
                .unwrap();
            assert_eq!(favorites.total, 2);
            assert_eq!(
                favorites
                    .tracks
                    .iter()
                    .map(|track| track.id)
                    .collect::<Vec<_>>(),
                library.db.order_sorted(None, true, "Ș", sort).unwrap()
            );
        }
    }

    #[test]
    fn downloaded_track_uses_music_folder_and_filename_as_metadata() {
        let path = Path::new("program")
            .join("Music")
            .join("YouTube Channel")
            .join("A Song.mp3");
        assert_eq!(
            downloaded_names(&path),
            ("A Song".to_owned(), "YouTube Channel".to_owned())
        );
    }

    #[test]
    fn playlist_download_stays_in_its_album_across_music_rescans_and_album_can_be_renamed() {
        let library = TestLibrary::new("playlist-album-rename");
        let path = library.mp3("Playlist Artist", "Song");
        library
            .db
            .import_downloaded_to_album(&path, "Road Mix")
            .unwrap();
        let track = library
            .db
            .page(0, 10, "", false, None)
            .unwrap()
            .tracks
            .remove(0);
        assert_eq!(track.album, "Road Mix");
        assert_eq!(
            library
                .db
                .scan_downloaded_music(&library.music)
                .unwrap()
                .updated,
            0
        );
        assert_eq!(library.db.get(track.id).unwrap().unwrap().album, "Road Mix");
        assert_eq!(
            library
                .db
                .rename_album(track.album_id, "Drive Mix")
                .unwrap(),
            "Drive Mix"
        );
        assert_eq!(
            library.db.get(track.id).unwrap().unwrap().album,
            "Drive Mix"
        );
        let audio = std::fs::read(&path).unwrap();
        std::fs::write(&path, audio.repeat(2)).unwrap();
        library.db.scan(library.music.to_str().unwrap()).unwrap();
        assert_eq!(
            library.db.get(track.id).unwrap().unwrap().album,
            "Drive Mix"
        );
        assert!(library.db.rename_album(track.album_id, " ").is_err());
        assert!(library
            .db
            .rename_album(track.album_id, &"x".repeat(121))
            .is_err());
    }

    #[test]
    fn music_scan_imports_new_files_only_once() {
        let directory =
            std::env::temp_dir().join(format!("aura-music-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let music = directory.join("Music");
        let track_directory = music.join("Channel Name");
        let second_track_directory = music.join("Another Artist");
        let nested_track_directory = music.join("Additional Folder").join("Nested Artist");
        std::fs::create_dir_all(&track_directory).unwrap();
        std::fs::create_dir_all(&second_track_directory).unwrap();
        std::fs::create_dir_all(&nested_track_directory).unwrap();
        let audio_path = track_directory.join("New Song.wav");
        let second_audio_path = second_track_directory.join("Second Song.wav");
        let nested_audio_path = nested_track_directory.join("Nested Song.wav");
        write_test_wav(&audio_path);
        write_test_wav(&second_audio_path);
        write_test_wav(&nested_audio_path);
        let database = Database::new(directory.join("library.sqlite3")).unwrap();

        let first = database.scan_downloaded_music(&music).unwrap();
        assert_eq!(first.discovered, 3);
        assert_eq!(first.updated, 3);
        let second = database.scan_downloaded_music(&music).unwrap();
        assert_eq!(second.discovered, 3);
        assert_eq!(second.updated, 0);
        let tracks = database.page(0, 10, "", false, None).unwrap();
        assert_eq!(tracks.total, 3);
        let imported_tracks: HashSet<_> = tracks
            .tracks
            .iter()
            .map(|track| (track.title.as_str(), track.artist.as_str()))
            .collect();
        assert!(imported_tracks.contains(&("New Song", "Channel Name")));
        assert!(imported_tracks.contains(&("Second Song", "Another Artist")));
        assert!(imported_tracks.contains(&("Nested Song", "Nested Artist")));
        let albums = database.albums("").unwrap();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].title, "Descarcate");
        assert_eq!(albums[0].track_count, 3);
        assert_eq!(
            database
                .page(0, 10, "", false, Some(albums[0].id))
                .unwrap()
                .total,
            3
        );

        drop(database);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn mp3_scan_repairs_existing_imports_and_preserves_user_data() {
        let library = TestLibrary::new("repair-download");
        let path = library.mp3("Autor cu diacritice șț", "Titlul melodiei");
        let mut tag = Tag::new(TagType::Id3v2);
        tag.set_title("Embedded title".into());
        tag.set_artist("Embedded artist".into());
        tag.set_album("Embedded album".into());
        tag.save_to_path(&path, WriteOptions::default()).unwrap();
        library.db.scan(library.music.to_str().unwrap()).unwrap();
        let original = library
            .db
            .page(0, 10, "", false, None)
            .unwrap()
            .tracks
            .remove(0);
        assert_eq!(original.title, "Embedded title");
        library.db.favorites(original.id).unwrap();
        library.db.note_play(original.id);

        let scan = library.db.scan_downloaded_music(&library.music).unwrap();
        assert_eq!((scan.updated, scan.errors), (1, 0));
        let track = library.db.get(original.id).unwrap().unwrap();
        assert_eq!(track.title, "Titlul melodiei");
        assert_eq!(track.artist, "Autor cu diacritice șț");
        assert_eq!(track.album, "Descarcate");
        assert!(track.is_favorite);
        assert_eq!(track.play_count, 1);

        let reopened = Database::new(library.directory.join("library.sqlite3")).unwrap();
        assert_eq!(
            reopened
                .scan_downloaded_music(&library.music)
                .unwrap()
                .updated,
            0
        );
        assert_eq!(reopened.page(0, 10, "", false, None).unwrap().total, 1);
        let albums = reopened.albums("").unwrap();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].title, "Descarcate");
    }

    #[test]
    fn modified_download_is_reindexed_without_duplicates() {
        let library = TestLibrary::new("modified-download");
        let path = library.mp3("Artist", "Song");
        library.db.scan_downloaded_music(&library.music).unwrap();
        let original = library
            .db
            .page(0, 10, "", false, None)
            .unwrap()
            .tracks
            .remove(0);
        let audio = std::fs::read(&path).unwrap();
        std::fs::write(&path, audio.repeat(2)).unwrap();

        let scan = library.db.scan_downloaded_music(&library.music).unwrap();
        assert_eq!((scan.updated, scan.errors), (1, 0));
        let tracks = library.db.page(0, 10, "", false, None).unwrap();
        assert_eq!(tracks.total, 1);
        assert_eq!(tracks.tracks[0].id, original.id);
        assert!(tracks.tracks[0].duration_ms > original.duration_ms);
        assert_eq!(
            library
                .db
                .scan_downloaded_music(&library.music)
                .unwrap()
                .updated,
            0
        );
    }

    #[test]
    fn returned_download_is_restored_with_the_same_id_and_favorite() {
        let library = TestLibrary::new("returned-download");
        let path = library.mp3("Artist", "Song");
        library.db.scan_downloaded_music(&library.music).unwrap();
        let track = library
            .db
            .page(0, 10, "", false, None)
            .unwrap()
            .tracks
            .remove(0);
        library.db.favorites(track.id).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            library
                .db
                .scan_downloaded_music(&library.music)
                .unwrap()
                .missing,
            1
        );
        assert_eq!(library.db.page(0, 10, "", false, None).unwrap().total, 0);

        write_test_mp3(&path);
        assert_eq!(
            library
                .db
                .scan_downloaded_music(&library.music)
                .unwrap()
                .updated,
            1
        );
        let restored = library.db.get(track.id).unwrap().unwrap();
        assert!(!restored.is_missing);
        assert!(restored.is_favorite);
        assert_eq!(
            library
                .db
                .page(0, 10, "", false, Some(track.album_id))
                .unwrap()
                .total,
            1
        );
    }

    #[test]
    fn scan_retries_invalid_audio_and_ignores_partial_files() {
        let library = TestLibrary::new("invalid-download");
        library.mp3("Artist", "Song");
        let broken = library.music.join("Artist").join("Broken.mp3");
        std::fs::write(&broken, b"Not audio").unwrap();
        std::fs::write(broken.with_extension("mp3.part"), b"Downloading").unwrap();
        let scan = library.db.scan_downloaded_music(&library.music).unwrap();
        assert_eq!((scan.discovered, scan.updated, scan.errors), (2, 1, 1));
        write_test_mp3(&broken);
        let scan = library.db.scan_downloaded_music(&library.music).unwrap();
        assert_eq!((scan.discovered, scan.updated, scan.errors), (2, 1, 0));
        assert_eq!(library.db.page(0, 10, "", false, None).unwrap().total, 2);
    }

    fn write_test_wav(path: &Path) {
        let sample_rate = 8_000_u32;
        let sample_count = 80_u32;
        let data_size = sample_count * 2;
        let mut wav = Vec::with_capacity(44 + data_size as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_size).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        wav.extend_from_slice(&2_u16.to_le_bytes());
        wav.extend_from_slice(&16_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        wav.resize(44 + data_size as usize, 0);
        std::fs::write(path, wav).unwrap();
    }
}
