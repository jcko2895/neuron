//! Instagram adapter — parses Meta's "Download Your Information" export.
//!
//! Structure:
//! - your_instagram_activity/messages/ — DMs
//! - logged_information/ — search history, activity
//! - connections/ — followers, following, contacts
//! - personal_information/ — profile data

use crate::common::{CommonRecord, TrustLevel};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

pub struct InstagramAdapter {
    user_name: String,
}

impl InstagramAdapter {
    pub fn new(user_name: &str) -> Self {
        Self { user_name: user_name.to_string() }
    }
}

impl super::SourceAdapter for InstagramAdapter {
    fn name(&self) -> &str { "Instagram" }
    fn platform(&self) -> &str { "instagram" }

    fn can_handle_file(&self, path: &Path) -> bool {
        path.join("your_instagram_activity").exists() || path.join("connections").exists()
    }

    fn extract_from_file(&self, path: &Path) -> Result<Vec<CommonRecord>, String> {
        let mut records = Vec::new();

        // DMs
        let msgs_dir = path.join("your_instagram_activity").join("messages").join("inbox");
        if msgs_dir.exists() {
            if let Ok(threads) = std::fs::read_dir(&msgs_dir) {
                for thread in threads.flatten() {
                    let thread_path = thread.path();
                    if !thread_path.is_dir() { continue; }
                    let thread_name = thread_path.file_name()
                        .and_then(|n| n.to_str()).unwrap_or("unknown").to_string();

                    if let Ok(files) = std::fs::read_dir(&thread_path) {
                        for file in files.flatten() {
                            let fp = file.path();
                            if fp.extension().and_then(|e| e.to_str()) != Some("json") { continue; }
                            if let Ok(data) = std::fs::read_to_string(&fp) {
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                                    if let Some(messages) = json.get("messages").and_then(|m| m.as_array()) {
                                        for msg in messages {
                                            let sender = msg.get("sender_name").and_then(|s| s.as_str()).unwrap_or("").to_string();
                                            let content = msg.get("content").and_then(|s| s.as_str()).unwrap_or("").to_string();
                                            if content.is_empty() { continue; }
                                            let ts_ms = msg.get("timestamp_ms").and_then(|t| t.as_u64()).unwrap_or(0);
                                            let timestamp = if ts_ms > 0 { Some(crate::adapters::facebook::format_unix_timestamp_pub(ts_ms / 1000)) } else { None };
                                            let is_user = sender.contains(&self.user_name);
                                            records.push(CommonRecord {
                                                content: content.clone(),
                                                timestamp,
                                                actor: Some(sender),
                                                is_user,
                                                source_file: fp.to_string_lossy().to_string(),
                                                source_type: "instagram_download_your_info".into(),
                                                trust_level: TrustLevel::Primary,
                                                content_hash: CommonRecord::compute_content_hash(&content),
                                                platform: "instagram".into(),
                                                thread_id: Some(thread_name.clone()),
                                                thread_name: Some(thread_name.clone()),
                                                account: None,
                                                metadata: serde_json::json!({}),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Likes, search history, etc from logged_information
        let logged = path.join("logged_information");
        if logged.exists() {
            // Search history
            let search_file = logged.join("search").join("search.json");
            if search_file.exists() {
                if let Ok(data) = std::fs::read_to_string(&search_file) {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                        if let Some(searches) = json.as_array().or_else(|| json.get("searches_user").and_then(|s| s.as_array())) {
                            for s in searches {
                                let title = s.get("title").or_else(|| s.get("string_map_data").and_then(|m| m.get("Search")).and_then(|v| v.get("value"))).and_then(|v| v.as_str()).unwrap_or("").to_string();
                                if title.is_empty() { continue; }
                                let ts = s.get("timestamp").and_then(|t| t.as_u64()).unwrap_or(0);
                                let timestamp = if ts > 0 { Some(crate::adapters::facebook::format_unix_timestamp_pub(ts)) } else { None };
                                records.push(CommonRecord {
                                    content: format!("[Instagram Search] {}", title),
                                    timestamp,
                                    actor: Some(self.user_name.clone()),
                                    is_user: true,
                                    source_file: search_file.to_string_lossy().to_string(),
                                    source_type: "instagram_search_history".into(),
                                    trust_level: TrustLevel::Primary,
                                    content_hash: CommonRecord::compute_content_hash(&title),
                                    platform: "instagram".into(),
                                    thread_id: None,
                                    thread_name: None,
                                    account: None,
                                    metadata: serde_json::json!({}),
                                });
                            }
                        }
                    }
                }
            }
        }

        info!(records = records.len(), "Instagram extraction complete");
        Ok(records)
    }

    fn discover_local(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let candidates = [
            PathBuf::from("D:/EVA/SUBSTRATE/data/raw/instagram"),
        ];
        for p in &candidates {
            if self.can_handle_file(p) { found.push(p.clone()); }
        }
        found
    }
}
