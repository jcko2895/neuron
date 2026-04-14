//! Browser History adapter — reads SQLite History databases from Chrome, Edge, Firefox, Safari.
//!
//! Browser history databases are locked while the browser is running,
//! so we copy the database to a temp file before reading.
//!
//! Chrome/Edge time format: microseconds since Jan 1, 1601 (Windows FILETIME).
//!   Convert: subtract 11644473600000000, divide by 1000000 for Unix seconds.
//!
//! Firefox time format: microseconds since Unix epoch.
//!   Convert: divide by 1000000 for Unix seconds.

use crate::common::{format_unix_timestamp, AccountContext, CommonRecord, TrustLevel};
use std::path::{Path, PathBuf};
use tracing::warn;

/// Which browser engine produced this history database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserKind {
    Chrome,
    Edge,
    Firefox,
    Safari,
}

impl BrowserKind {
    fn platform_str(&self) -> &'static str {
        match self {
            BrowserKind::Chrome => "chrome",
            BrowserKind::Edge => "edge",
            BrowserKind::Firefox => "firefox",
            BrowserKind::Safari => "safari",
        }
    }

    fn display_name(&self) -> &'static str {
        match self {
            BrowserKind::Chrome => "Chrome",
            BrowserKind::Edge => "Edge",
            BrowserKind::Firefox => "Firefox",
            BrowserKind::Safari => "Safari",
        }
    }

    fn source_type(&self) -> &'static str {
        match self {
            BrowserKind::Chrome => "chrome_history_sqlite",
            BrowserKind::Edge => "edge_history_sqlite",
            BrowserKind::Firefox => "firefox_history_sqlite",
            BrowserKind::Safari => "safari_history_sqlite",
        }
    }
}

pub struct BrowserHistoryAdapter {
    kind: BrowserKind,
}

impl BrowserHistoryAdapter {
    pub fn new(kind: BrowserKind) -> Self {
        Self { kind }
    }

    /// Copy the database file to a temporary location so we don't fight browser locks.
    fn copy_to_temp(path: &Path) -> Result<PathBuf, String> {
        let temp_dir = std::env::temp_dir();
        let temp_path = temp_dir.join("neuron_browser_history_copy.sqlite");
        std::fs::copy(path, &temp_path)
            .map_err(|e| format!("Failed to copy {} to temp: {}", path.display(), e))?;
        Ok(temp_path)
    }

    /// Convert Chrome/Edge timestamp (microseconds since 1601-01-01) to Unix seconds.
    fn chromium_time_to_unix(chromium_us: i64) -> u64 {
        const EPOCH_DIFF_US: i64 = 11_644_473_600_000_000;
        let unix_us = chromium_us - EPOCH_DIFF_US;
        if unix_us < 0 {
            return 0;
        }
        (unix_us / 1_000_000) as u64
    }

    /// Convert Firefox timestamp (microseconds since Unix epoch) to Unix seconds.
    fn firefox_time_to_unix(firefox_us: i64) -> u64 {
        if firefox_us < 0 {
            return 0;
        }
        (firefox_us / 1_000_000) as u64
    }

    /// Extract records from a Chromium-based browser (Chrome/Edge).
    fn extract_chromium(&self, db_path: &Path, original_path: &Path) -> Result<Vec<CommonRecord>, String> {
        let temp_path = Self::copy_to_temp(db_path)?;
        let conn = rusqlite::Connection::open_with_flags(
            &temp_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| format!("Failed to open SQLite: {}", e))?;

        let mut stmt = conn
            .prepare(
                "SELECT u.url, u.title, u.visit_count, v.visit_time
                 FROM urls u
                 JOIN visits v ON u.id = v.url
                 ORDER BY v.visit_time DESC",
            )
            .map_err(|e| format!("Failed to prepare query: {}", e))?;

        let mut records = Vec::new();

        let rows = stmt
            .query_map([], |row| {
                let url: String = row.get(0)?;
                let title: String = row.get(1)?;
                let visit_count: i64 = row.get(2)?;
                let visit_time: i64 = row.get(3)?;
                Ok((url, title, visit_count, visit_time))
            })
            .map_err(|e| format!("Query failed: {}", e))?;

        for row in rows {
            let (url, title, visit_count, visit_time) = match row {
                Ok(r) => r,
                Err(e) => {
                    warn!(error = %e, "skipping malformed row");
                    continue;
                }
            };

            let unix_secs = Self::chromium_time_to_unix(visit_time);
            let timestamp = if unix_secs > 0 {
                Some(format_unix_timestamp(unix_secs))
            } else {
                None
            };

            let content = if title.is_empty() {
                url.clone()
            } else {
                format!("{} — {}", title, url)
            };

            let content_hash = CommonRecord::compute_content_hash(&content);

            records.push(CommonRecord {
                content,
                timestamp,
                actor: None,
                is_user: true,
                source_file: original_path.to_string_lossy().to_string(),
                source_type: self.kind.source_type().to_string(),
                trust_level: TrustLevel::Primary,
                content_hash,
                platform: self.kind.platform_str().to_string(),
                thread_id: None,
                thread_name: None,
                account: Some(AccountContext {
                    platform: self.kind.platform_str().to_string(),
                    account_id: "default".to_string(),
                    display_name: self.kind.display_name().to_string(),
                    account_type: "personal".to_string(),
                    persona_notes: None,
                }),
                metadata: serde_json::json!({
                    "url": url,
                    "title": title,
                    "visit_count": visit_count,
                }),
            });
        }

        // Clean up temp file
        let _ = std::fs::remove_file(&temp_path);

        Ok(records)
    }

    /// Extract records from a Firefox places.sqlite database.
    fn extract_firefox(&self, db_path: &Path, original_path: &Path) -> Result<Vec<CommonRecord>, String> {
        let temp_path = Self::copy_to_temp(db_path)?;
        let conn = rusqlite::Connection::open_with_flags(
            &temp_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| format!("Failed to open SQLite: {}", e))?;

        let mut stmt = conn
            .prepare(
                "SELECT p.url, p.title, p.visit_count, h.visit_date
                 FROM moz_places p
                 JOIN moz_historyvisits h ON p.id = h.place_id
                 ORDER BY h.visit_date DESC",
            )
            .map_err(|e| format!("Failed to prepare query: {}", e))?;

        let mut records = Vec::new();

        let rows = stmt
            .query_map([], |row| {
                let url: String = row.get(0)?;
                let title: Option<String> = row.get(1)?;
                let visit_count: i64 = row.get(2)?;
                let visit_date: i64 = row.get(3)?;
                Ok((url, title, visit_count, visit_date))
            })
            .map_err(|e| format!("Query failed: {}", e))?;

        for row in rows {
            let (url, title, visit_count, visit_date) = match row {
                Ok(r) => r,
                Err(e) => {
                    warn!(error = %e, "skipping malformed row");
                    continue;
                }
            };

            let unix_secs = Self::firefox_time_to_unix(visit_date);
            let timestamp = if unix_secs > 0 {
                Some(format_unix_timestamp(unix_secs))
            } else {
                None
            };

            let title_str = title.unwrap_or_default();
            let content = if title_str.is_empty() {
                url.clone()
            } else {
                format!("{} — {}", title_str, url)
            };

            let content_hash = CommonRecord::compute_content_hash(&content);

            records.push(CommonRecord {
                content,
                timestamp,
                actor: None,
                is_user: true,
                source_file: original_path.to_string_lossy().to_string(),
                source_type: self.kind.source_type().to_string(),
                trust_level: TrustLevel::Primary,
                content_hash,
                platform: self.kind.platform_str().to_string(),
                thread_id: None,
                thread_name: None,
                account: Some(AccountContext {
                    platform: self.kind.platform_str().to_string(),
                    account_id: "default".to_string(),
                    display_name: self.kind.display_name().to_string(),
                    account_type: "personal".to_string(),
                    persona_notes: None,
                }),
                metadata: serde_json::json!({
                    "url": url,
                    "title": title_str,
                    "visit_count": visit_count,
                }),
            });
        }

        // Clean up temp file
        let _ = std::fs::remove_file(&temp_path);

        Ok(records)
    }

    /// Extract Safari history.
    /// Safari uses history_items + history_visits tables.
    /// Timestamps are seconds since Apple epoch (2001-01-01).
    fn extract_safari(&self, db_path: &Path, original_path: &Path) -> Result<Vec<CommonRecord>, String> {
        const APPLE_EPOCH: u64 = 978307200;

        // Copy to temp to avoid lock
        let temp_path = std::env::temp_dir().join("neuron_safari_history_copy.db");
        std::fs::copy(db_path, &temp_path)
            .map_err(|e| format!("Failed to copy Safari history: {}", e))?;

        let conn = rusqlite::Connection::open_with_flags(
            &temp_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ).map_err(|e| format!("Failed to open Safari history: {}", e))?;

        let mut stmt = conn.prepare(
            "SELECT hi.url, hi.domain_expansion, hv.title, hv.visit_time
             FROM history_visits hv
             JOIN history_items hi ON hv.history_item = hi.id
             WHERE hv.visit_time IS NOT NULL
             ORDER BY hv.visit_time DESC"
        ).map_err(|e| format!("Safari query failed: {}", e))?;

        let mut records = Vec::new();

        let rows = stmt.query_map([], |row| {
            let url: String = row.get(0)?;
            let domain: Option<String> = row.get(1)?;
            let title: Option<String> = row.get(2)?;
            let visit_time: f64 = row.get(3)?;
            Ok((url, domain, title, visit_time))
        }).map_err(|e| format!("Query failed: {}", e))?;

        for row in rows {
            let (url, _domain, title, visit_time) = match row {
                Ok(r) => r,
                Err(_) => continue,
            };

            let unix_secs = (visit_time as u64) + APPLE_EPOCH;
            let timestamp = Some(format_unix_timestamp(unix_secs));

            let title = title.unwrap_or_default();
            let content = if title.is_empty() {
                url.clone()
            } else {
                format!("{} — {}", title, url)
            };

            records.push(CommonRecord {
                content: content.clone(),
                timestamp,
                actor: None,
                is_user: true,
                source_file: original_path.to_string_lossy().to_string(),
                source_type: "safari_history".into(),
                trust_level: TrustLevel::Primary,
                content_hash: CommonRecord::compute_content_hash(&content),
                platform: "safari".into(),
                thread_id: None,
                thread_name: None,
                account: None,
                metadata: serde_json::json!({ "url": url }),
            });
        }

        let _ = std::fs::remove_file(&temp_path);
        Ok(records)
    }
}

impl super::SourceAdapter for BrowserHistoryAdapter {
    fn name(&self) -> &str {
        self.kind.display_name()
    }

    fn platform(&self) -> &str {
        self.kind.platform_str()
    }

    fn can_handle_file(&self, path: &Path) -> bool {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        match self.kind {
            BrowserKind::Chrome | BrowserKind::Edge => name == "History",
            BrowserKind::Firefox => name == "places.sqlite",
            BrowserKind::Safari => name == "History.db",
        }
    }

    fn extract_from_file(&self, path: &Path) -> Result<Vec<CommonRecord>, String> {
        match self.kind {
            BrowserKind::Chrome | BrowserKind::Edge => self.extract_chromium(path, path),
            BrowserKind::Firefox => self.extract_firefox(path, path),
            BrowserKind::Safari => self.extract_safari(path, path),
        }
    }

    fn discover_local(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();

        let home = dirs_next::home_dir().unwrap_or_default();

        let candidates: Vec<PathBuf> = match self.kind {
            BrowserKind::Edge => vec![
                // Windows
                home.join("AppData/Local/Microsoft/Edge/User Data/Default/History"),
            ],
            BrowserKind::Chrome => vec![
                // Windows
                home.join("AppData/Local/Google/Chrome/User Data/Default/History"),
                // macOS
                home.join("Library/Application Support/Google/Chrome/Default/History"),
                // Linux
                home.join(".config/google-chrome/Default/History"),
            ],
            BrowserKind::Firefox => {
                // Firefox profiles are in a randomly-named directory
                let mut ff_candidates = Vec::new();
                let profiles_dir = if cfg!(windows) {
                    home.join("AppData/Roaming/Mozilla/Firefox/Profiles")
                } else if cfg!(target_os = "macos") {
                    home.join("Library/Application Support/Firefox/Profiles")
                } else {
                    home.join(".mozilla/firefox")
                };
                if let Ok(entries) = std::fs::read_dir(&profiles_dir) {
                    for entry in entries.flatten() {
                        let places = entry.path().join("places.sqlite");
                        if places.exists() {
                            ff_candidates.push(places);
                        }
                    }
                }
                ff_candidates
            }
            BrowserKind::Safari => vec![
                home.join("Library/Safari/History.db"),
            ],
        };

        for path in candidates {
            if path.exists() {
                found.push(path);
            }
        }

        found
    }
}
