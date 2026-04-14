//! Spotify adapter — parses Spotify's "Download your data" export.
//!
//! Spotify export structure:
//! ```
//! my_spotify_data/
//!   StreamingHistory_music_0.json   — listening history (recent)
//!   StreamingHistory_music_1.json   — continued...
//!   StreamingHistory_podcast_0.json — podcast listening
//!   Playlist1.json                  — playlists
//!   YourLibrary.json                — saved tracks/albums/artists
//!   SearchQueries.json              — search history
//!   Inferences.json                 — Spotify's inferences about you
//!   Identity.json                   — account info
//! ```
//!
//! Extended streaming history (request separately) has richer data:
//! ```json
//! {"ts": "2024-01-15T22:30:00Z", "ms_played": 234000,
//!  "master_metadata_track_name": "Song", "master_metadata_album_artist_name": "Artist",
//!  "master_metadata_album_album_name": "Album", "spotify_track_uri": "spotify:track:..."}
//! ```

use crate::common::{CommonRecord, TrustLevel};
use std::path::{Path, PathBuf};
use tracing::info;

pub struct SpotifyAdapter {
    user_name: String,
}

impl SpotifyAdapter {
    pub fn new(user_name: &str) -> Self {
        Self { user_name: user_name.to_string() }
    }

    /// Collect all streaming history JSON files from root and subdirectories.
    fn find_history_files(dir: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return files,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(Self::find_history_files(&path));
            } else {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                // Match both formats:
                // Standard:  StreamingHistory_music_0.json
                // Extended:  Streaming_History_Audio_2012-2013_0.json
                if (name.starts_with("StreamingHistory") || name.starts_with("Streaming_History"))
                    && name.ends_with(".json")
                {
                    files.push(path);
                }
            }
        }
        files
    }

    fn extract_streaming_history(&self, root: &Path) -> Vec<CommonRecord> {
        let mut records = Vec::new();
        let history_files = Self::find_history_files(root);

        for path in &history_files {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

            let is_podcast = name.contains("podcast") || name.contains("Video");

            let data = match std::fs::read_to_string(path) {
                Ok(d) => d,
                Err(_) => continue,
            };

            let items: Vec<serde_json::Value> = match serde_json::from_str(&data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            for item in &items {
                // Standard format: trackName/artistName
                // Extended format: master_metadata_track_name/master_metadata_album_artist_name
                let track = item.get("trackName")
                    .or_else(|| item.get("master_metadata_track_name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let artist = item.get("artistName")
                    .or_else(|| item.get("master_metadata_album_artist_name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let album = item.get("master_metadata_album_album_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                // Podcast/episode fields
                let episode = item.get("episode_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let show = item.get("episode_show_name").and_then(|v| v.as_str()).unwrap_or("").to_string();

                // Timestamp: "endTime" (standard) or "ts" (extended, UTC)
                let timestamp = item.get("endTime")
                    .or_else(|| item.get("ts"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let ms_played = item.get("msPlayed")
                    .or_else(|| item.get("ms_played"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                // Extended metadata
                let platform_device = item.get("platform").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let shuffle = item.get("shuffle").and_then(|v| v.as_bool()).unwrap_or(false);
                let skipped = item.get("skipped").and_then(|v| v.as_bool()).unwrap_or(false);
                let offline = item.get("offline").and_then(|v| v.as_bool()).unwrap_or(false);
                let reason_start = item.get("reason_start").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let reason_end = item.get("reason_end").and_then(|v| v.as_str()).unwrap_or("").to_string();

                // Skip entries with no track/artist/episode
                if track.is_empty() && artist.is_empty() && episode.is_empty() { continue; }

                // Skip very short plays (< 5 seconds) — likely skips
                if ms_played > 0 && ms_played < 5000 { continue; }

                let mins = ms_played / 60000;
                let secs = (ms_played % 60000) / 1000;

                let (_label, content) = if !episode.is_empty() {
                    ("Podcast", format!("[Spotify Podcast] {} — {} ({}:{:02})", episode, show, mins, secs))
                } else if !album.is_empty() {
                    ("Listened", format!("[Spotify] {} — {} ({}) [{}:{:02}]", track, artist, album, mins, secs))
                } else {
                    ("Listened", format!("[Spotify] {} — {} [{}:{:02}]", track, artist, mins, secs))
                };

                let uri = item.get("spotify_track_uri")
                    .or_else(|| item.get("spotify_episode_uri"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                records.push(CommonRecord {
                    content: content.clone(),
                    timestamp,
                    actor: Some(self.user_name.clone()),
                    is_user: true,
                    source_file: path.to_string_lossy().to_string(),
                    source_type: if is_podcast { "spotify_podcast_history" } else { "spotify_streaming_history" }.into(),
                    trust_level: TrustLevel::Primary,
                    content_hash: CommonRecord::compute_content_hash(&content),
                    platform: "spotify".into(),
                    thread_id: None,
                    thread_name: None,
                    account: None,
                    metadata: serde_json::json!({
                        "artist": artist,
                        "track": track,
                        "album": album,
                        "ms_played": ms_played,
                        "uri": uri,
                        "device_platform": platform_device,
                        "shuffle": shuffle,
                        "skipped": skipped,
                        "offline": offline,
                        "reason_start": reason_start,
                        "reason_end": reason_end,
                    }),
                });
            }
        }

        records
    }

    fn extract_library(&self, root: &Path) -> Vec<CommonRecord> {
        let mut records = Vec::new();
        let lib_file = if root.join("YourLibrary.json").exists() {
            root.join("YourLibrary.json")
        } else {
            root.join("Spotify Account Data").join("YourLibrary.json")
        };
        if !lib_file.exists() { return records; }

        let data = match std::fs::read_to_string(&lib_file) {
            Ok(d) => d,
            Err(_) => return records,
        };

        let json: serde_json::Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(_) => return records,
        };

        // Saved tracks
        if let Some(tracks) = json.get("tracks").and_then(|v| v.as_array()) {
            for t in tracks {
                let name = t.get("track").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let artist = t.get("artist").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let album = t.get("album").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if name.is_empty() { continue; }

                let content = format!("[Spotify Library] {} — {} ({})", name, artist, album);
                records.push(CommonRecord {
                    content: content.clone(),
                    timestamp: None,
                    actor: Some(self.user_name.clone()),
                    is_user: true,
                    source_file: lib_file.to_string_lossy().to_string(),
                    source_type: "spotify_library".into(),
                    trust_level: TrustLevel::Primary,
                    content_hash: CommonRecord::compute_content_hash(&content),
                    platform: "spotify".into(),
                    thread_id: None,
                    thread_name: None,
                    account: None,
                    metadata: serde_json::json!({
                        "artist": artist,
                        "track": name,
                        "album": album,
                    }),
                });
            }
        }

        // Saved artists
        if let Some(artists) = json.get("artists").and_then(|v| v.as_array()) {
            for a in artists {
                let name = a.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if name.is_empty() { continue; }
                let content = format!("[Spotify Followed Artist] {}", name);
                records.push(CommonRecord {
                    content: content.clone(),
                    timestamp: None,
                    actor: Some(self.user_name.clone()),
                    is_user: true,
                    source_file: lib_file.to_string_lossy().to_string(),
                    source_type: "spotify_library".into(),
                    trust_level: TrustLevel::Primary,
                    content_hash: CommonRecord::compute_content_hash(&content),
                    platform: "spotify".into(),
                    thread_id: None,
                    thread_name: None,
                    account: None,
                    metadata: serde_json::json!({}),
                });
            }
        }

        records
    }

    fn extract_search_history(&self, root: &Path) -> Vec<CommonRecord> {
        let mut records = Vec::new();
        let search_file = if root.join("SearchQueries.json").exists() {
            root.join("SearchQueries.json")
        } else {
            root.join("Spotify Account Data").join("SearchQueries.json")
        };
        if !search_file.exists() { return records; }

        let data = match std::fs::read_to_string(&search_file) {
            Ok(d) => d,
            Err(_) => return records,
        };

        let items: Vec<serde_json::Value> = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(_) => return records,
        };

        for item in &items {
            let query = item.get("searchQuery").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let timestamp = item.get("searchTime").and_then(|v| v.as_str()).map(|s| s.to_string());
            if query.is_empty() { continue; }

            let content = format!("[Spotify Search] {}", query);
            records.push(CommonRecord {
                content: content.clone(),
                timestamp,
                actor: Some(self.user_name.clone()),
                is_user: true,
                source_file: search_file.to_string_lossy().to_string(),
                source_type: "spotify_search_history".into(),
                trust_level: TrustLevel::Primary,
                content_hash: CommonRecord::compute_content_hash(&content),
                platform: "spotify".into(),
                thread_id: None,
                thread_name: None,
                account: None,
                metadata: serde_json::json!({}),
            });
        }

        records
    }
}

impl super::SourceAdapter for SpotifyAdapter {
    fn name(&self) -> &str { "Spotify" }
    fn platform(&self) -> &str { "spotify" }

    fn can_handle_file(&self, path: &Path) -> bool {
        if !path.is_dir() { return false; }
        // Direct: StreamingHistory*.json in root
        // Subdirectory: "Spotify Account Data/" or "Spotify Extended Streaming History/"
        !Self::find_history_files(path).is_empty()
            || path.join("Spotify Account Data").exists()
            || path.join("Spotify Extended Streaming History").exists()
    }

    fn extract_from_file(&self, path: &Path) -> Result<Vec<CommonRecord>, String> {
        let mut all = Vec::new();
        all.extend(self.extract_streaming_history(path));
        all.extend(self.extract_library(path));
        all.extend(self.extract_search_history(path));
        info!(total = all.len(), "Spotify extraction complete");
        Ok(all)
    }

    fn discover_local(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let candidates = [
            PathBuf::from("D:/EVA/SUBSTRATE/data/raw/spotify"),
        ];
        if let Some(downloads) = dirs_next::download_dir() {
            let extra = [
                downloads.join("my_spotify_data"),
                downloads.join("Spotify"),
            ];
            for p in &extra {
                if self.can_handle_file(p) { found.push(p.clone()); }
            }
        }
        for p in &candidates {
            if self.can_handle_file(p) { found.push(p.clone()); }
        }
        found
    }
}
