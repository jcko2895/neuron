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

    fn extract_streaming_history(&self, root: &Path) -> Vec<CommonRecord> {
        let mut records = Vec::new();

        // Find all StreamingHistory*.json files
        let entries = match std::fs::read_dir(root) {
            Ok(e) => e,
            Err(_) => return records,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();

            if !name.starts_with("StreamingHistory") || !name.ends_with(".json") {
                continue;
            }

            let is_podcast = name.contains("podcast");

            let data = match std::fs::read_to_string(&path) {
                Ok(d) => d,
                Err(_) => continue,
            };

            let items: Vec<serde_json::Value> = match serde_json::from_str(&data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            for item in &items {
                // Standard format
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

                // Timestamp: "endTime" (standard) or "ts" (extended)
                let timestamp = item.get("endTime")
                    .or_else(|| item.get("ts"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let ms_played = item.get("msPlayed")
                    .or_else(|| item.get("ms_played"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                if track.is_empty() && artist.is_empty() { continue; }

                // Skip very short plays (< 5 seconds) — likely skips
                if ms_played > 0 && ms_played < 5000 { continue; }

                let label = if is_podcast { "Podcast" } else { "Listened" };
                let content = if !album.is_empty() {
                    format!("[Spotify {}] {} — {} ({})", label, track, artist, album)
                } else {
                    format!("[Spotify {}] {} — {}", label, track, artist)
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
                    }),
                });
            }
        }

        records
    }

    fn extract_library(&self, root: &Path) -> Vec<CommonRecord> {
        let mut records = Vec::new();
        let lib_file = root.join("YourLibrary.json");
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
        let search_file = root.join("SearchQueries.json");
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
        // Spotify export has StreamingHistory*.json files
        if path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if name.starts_with("StreamingHistory") && name.ends_with(".json") {
                        return true;
                    }
                }
            }
        }
        // Or a single streaming history file
        if path.is_file() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                return name.starts_with("StreamingHistory") && name.ends_with(".json");
            }
        }
        false
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
        // Common Spotify export locations
        if let Some(downloads) = dirs_next::download_dir() {
            let candidates = [
                downloads.join("my_spotify_data"),
                downloads.join("Spotify"),
            ];
            for p in &candidates {
                if self.can_handle_file(p) { found.push(p.clone()); }
            }
        }
        found
    }
}
