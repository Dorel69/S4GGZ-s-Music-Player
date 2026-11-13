use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Platform {
    #[default]
    YouTube,
    SoundCloud,
}

impl Platform {
    pub fn label(self) -> &'static str {
        match self {
            Self::YouTube => "YouTube",
            Self::SoundCloud => "SoundCloud",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SearchResult {
    pub title: String,
    pub artist: String,
    pub duration: String,
    pub page_url: String,
    pub download_url: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PlaylistResult {
    pub title: String,
    pub tracks: Vec<SearchResult>,
}

/// Resolve a playlist into stable per-track page URLs before downloads start.
/// Keeping each track as an ordinary download job preserves cancellation,
/// retries, deduplication, and the existing serial transfer queue.
pub fn playlist(
    url: &str,
    cancel: &crate::process_runner::Cancellation,
    browser: Option<&str>,
) -> Result<PlaylistResult, String> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("Introdu un link HTTP sau HTTPS valid pentru playlist.".into());
    }
    let mut command = crate::yt_dlp::command_with_cookies(cancel, browser)?;
    command
        .args([
            "--no-warnings",
            "--no-color",
            "--skip-download",
            "--flat-playlist",
            "--dump-single-json",
            "--playlist-end",
            "200",
            "--",
        ])
        .arg(url);
    let output = crate::process_runner::run(
        &mut command,
        std::time::Duration::from_secs(60),
        cancel,
        None,
    )?;
    if !output.status.success() {
        return Err(format!(
            "Nu am putut citi playlistul: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let response: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Răspuns JSON yt-dlp invalid: {error}"))?;
    parse_playlist_response(&response)
}

fn parse_playlist_response(response: &Value) -> Result<PlaylistResult, String> {
    let title = response["title"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Playlist YouTube")
        .to_owned();
    let tracks = parse_playlist_entries(response)?;
    Ok(PlaylistResult { title, tracks })
}

fn parse_playlist_entries(response: &Value) -> Result<Vec<SearchResult>, String> {
    let entries = response["entries"]
        .as_array()
        .ok_or("Linkul nu conține un playlist citibil")?;
    let mut tracks = Vec::new();
    for entry in entries {
        let id = entry["id"].as_str().or_else(|| entry["url"].as_str());
        let page_url = entry["webpage_url"]
            .as_str()
            .or_else(|| entry["url"].as_str())
            .filter(|value| value.starts_with("http://") || value.starts_with("https://"))
            .map(str::to_owned)
            .or_else(|| {
                id.filter(|id| !id.starts_with("http://") && !id.starts_with("https://"))
                    .map(|id| format!("https://www.youtube.com/watch?v={id}"))
            });
        let Some(page_url) = page_url else { continue };
        let title = entry["title"]
            .as_str()
            .filter(|value| !value.is_empty())
            .unwrap_or("Fără titlu")
            .to_owned();
        let artist = ["channel", "uploader", "artist", "creator"]
            .iter()
            .find_map(|field| entry[*field].as_str())
            .unwrap_or("Artist necunoscut")
            .to_owned();
        tracks.push(SearchResult {
            title,
            artist,
            duration: String::new(),
            page_url: page_url.clone(),
            download_url: Some(page_url),
        });
    }
    if tracks.is_empty() {
        return Err("Playlistul nu conține melodii disponibile.".into());
    }
    Ok(tracks)
}

pub fn search(
    platform: Platform,
    query: &str,
    cancel: &crate::process_runner::Cancellation,
    browser: Option<&str>,
) -> Result<Vec<SearchResult>, String> {
    let prefix = search_prefix(platform);
    let mut command = crate::yt_dlp::command_with_cookies(cancel, browser)?;
    command
        .args([
            "--no-warnings",
            "--no-color",
            "--skip-download",
            "--flat-playlist",
            "--dump-single-json",
            "--playlist-end",
            "10",
        ])
        .arg(format!("{prefix}{query}"));
    let output = crate::process_runner::run(
        &mut command,
        std::time::Duration::from_secs(60),
        cancel,
        None,
    )?;
    if !output.status.success() {
        return Err(format!(
            "Căutarea a eșuat: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let response: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Răspuns JSON yt-dlp invalid: {error}"))?;
    parse_results(platform, &response)
}

fn search_prefix(platform: Platform) -> &'static str {
    match platform {
        Platform::YouTube => "ytsearch10:",
        // SoundCloud's yt-dlp extractor only recognizes `scsearch:`; unlike
        // YouTube's extractor, it does not parse a count suffix here.
        Platform::SoundCloud => "scsearch:",
    }
}

/// Return a match only when SoundCloud/yt-dlp reports that its owner enabled downloads.
pub fn soundcloud_downloadable_match(
    title: &str,
    artist: &str,
    cancel: &crate::process_runner::Cancellation,
) -> Result<Option<SearchResult>, String> {
    let query = format!("{title} {artist}");
    let Some(result) = search(Platform::SoundCloud, &query, cancel, None)?
        .into_iter()
        .find(|result| same_track(&result.title, &result.artist, title, artist))
    else {
        return Ok(None);
    };
    let mut command = crate::yt_dlp::command_with_cookies(cancel, None)?;
    command
        .args([
            "--no-warnings",
            "--no-color",
            "--no-playlist",
            "--skip-download",
            "--dump-single-json",
            "--",
        ])
        .arg(&result.page_url);
    let output = crate::process_runner::run(
        &mut command,
        std::time::Duration::from_secs(60),
        cancel,
        None,
    )?;
    if !output.status.success() {
        return Ok(None);
    }
    let metadata: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Metadatele SoundCloud nu sunt JSON valid: {error}"))?;
    Ok(has_owner_enabled_download(&metadata).then_some(result))
}

fn has_owner_enabled_download(metadata: &Value) -> bool {
    metadata["formats"].as_array().is_some_and(|formats| {
        formats
            .iter()
            .any(|format| format["format_id"].as_str() == Some("download"))
    })
}

fn same_track(found_title: &str, found_artist: &str, title: &str, artist: &str) -> bool {
    fn key(value: &str) -> String {
        value
            .chars()
            .filter(|character| character.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    }
    !title.trim().is_empty()
        && !artist.trim().is_empty()
        && key(found_title) == key(title)
        && key(found_artist) == key(artist)
}

fn parse_results(platform: Platform, response: &Value) -> Result<Vec<SearchResult>, String> {
    let entries = response["entries"]
        .as_array()
        .ok_or("Platforma nu a returnat rezultate")?;
    Ok(entries
        .iter()
        .filter_map(|entry| {
            let result_url = entry["webpage_url"]
                .as_str()
                .or_else(|| entry["url"].as_str());
            let page_url = match result_url {
                Some(url) if url.starts_with("http://") || url.starts_with("https://") => {
                    url.to_owned()
                }
                _ if platform == Platform::YouTube => {
                    let id = entry["id"].as_str().or_else(|| entry["url"].as_str())?;
                    format!("https://www.youtube.com/watch?v={id}")
                }
                _ => return None,
            };
            Some(SearchResult {
                title: text(&entry["title"]),
                artist: entry["channel"]
                    .as_str()
                    .or_else(|| entry["uploader"].as_str())
                    .or_else(|| entry["artist"].as_str())
                    .unwrap_or("Artist necunoscut")
                    .to_owned(),
                duration: entry["duration"]
                    .as_f64()
                    .map(format_seconds)
                    .unwrap_or_default(),
                download_url: Some(page_url.clone()),
                page_url,
            })
        })
        .take(10)
        .collect())
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or("Fără titlu").to_owned()
}

fn format_seconds(duration: f64) -> String {
    let seconds = duration.max(0.0) as u64;
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::format_seconds;

    #[test]
    fn formats_search_durations() {
        assert_eq!(format_seconds(185.9), "3:05");
        assert_eq!(format_seconds(0.0), "0:00");
        assert_eq!(format_seconds(3723.0), "1:02:03");
    }

    #[test]
    fn creates_youtube_url_for_flat_search_ids() {
        let response = serde_json::json!({
            "entries": [{"id": "video123", "title": "Test track", "duration": 185.0}]
        });
        let results = super::parse_results(super::Platform::YouTube, &response).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].page_url,
            "https://www.youtube.com/watch?v=video123"
        );
        assert_eq!(
            results[0].download_url.as_deref(),
            Some(results[0].page_url.as_str())
        );
    }

    #[test]
    fn uses_channel_name_as_download_author() {
        let response = serde_json::json!({
            "entries": [{
                "id": "video123",
                "title": "Song",
                "artist": "Performer",
                "channel": "YouTube Channel"
            }]
        });
        let results = super::parse_results(super::Platform::YouTube, &response).unwrap();
        assert_eq!(results[0].artist, "YouTube Channel");
    }

    #[test]
    fn playlist_entries_preserve_order_and_skip_unavailable_items() {
        let response = serde_json::json!({"entries": [
            {"id": "first", "title": "First", "channel": "Artist A"},
            null,
            {"webpage_url": "https://soundcloud.com/artist/second", "title": "Second", "uploader": "Artist B"}
        ]});
        let tracks = super::parse_playlist_entries(&response).unwrap();
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].page_url, "https://www.youtube.com/watch?v=first");
        assert_eq!(tracks[0].title, "First");
        assert_eq!(tracks[1].page_url, "https://soundcloud.com/artist/second");
        assert_eq!(tracks[1].artist, "Artist B");
    }

    #[test]
    fn empty_playlist_is_reported_clearly() {
        assert!(super::parse_playlist_entries(&serde_json::json!({"entries": []})).is_err());
    }

    #[test]
    fn soundcloud_matching_requires_both_normalized_title_and_artist() {
        assert!(super::same_track(
            "Song!",
            "The Artist",
            "song",
            "the artist"
        ));
        assert!(!super::same_track(
            "Song",
            "Other Artist",
            "Song",
            "The Artist"
        ));
        assert!(!super::same_track("Song", "The Artist", "", "The Artist"));
    }

    #[test]
    fn soundcloud_fallback_requires_the_platform_download_format() {
        assert!(super::has_owner_enabled_download(&serde_json::json!({
            "formats": [{"format_id":"http"}, {"format_id":"download"}]
        })));
        assert!(!super::has_owner_enabled_download(&serde_json::json!({
            "formats": [{"format_id":"http"}]
        })));
    }

    #[test]
    fn uses_the_supported_soundcloud_search_prefix() {
        assert_eq!(
            super::search_prefix(super::Platform::SoundCloud),
            "scsearch:"
        );
        assert_eq!(
            super::search_prefix(super::Platform::YouTube),
            "ytsearch10:"
        );
    }

    #[test]
    fn playlist_title_is_preserved_for_album_import() {
        let response = serde_json::json!({"title":"Road Mix","entries":[{"id":"x"}]});
        let playlist = super::parse_playlist_response(&response).unwrap();
        assert_eq!(playlist.title, "Road Mix");
        assert_eq!(playlist.tracks.len(), 1);
    }
}
