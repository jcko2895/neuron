//! Facebook Takeout adapter — parses raw Facebook data export.
//!
//! Facebook Takeout format:
//! ```
//! your_facebook_activity/
//!   messages/
//!     archived_threads/
//!       {thread_name}_{thread_id}/
//!         message_1.json
//!         message_2.json
//!         ...
//!     inbox/
//!       {thread_name}_{thread_id}/
//!         message_1.json
//!         ...
//! ```
//!
//! Each message_*.json contains:
//! ```json
//! {
//!   "participants": [{"name": "..."}],
//!   "messages": [
//!     {
//!       "sender_name": "...",
//!       "timestamp_ms": 1505254406168,
//!       "content": "...",
//!       "photos": [...],
//!       "reactions": [...]
//!     }
//!   ]
//! }
//! ```
//!
//! Facebook encodes names in latin-1 escaped as UTF-8.
//! "Nicholas Wilson Towne" might appear as "Nicholas Wilson Towne" or with
//! mojibake characters that need decoding.

use crate::common::{format_unix_timestamp, AccountContext, CommonRecord, TrustLevel};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

pub struct FacebookAdapter {
    /// The user's Facebook display name (for is_user detection)
    user_name: String,
}

impl FacebookAdapter {
    pub fn new(user_name: &str) -> Self {
        Self {
            user_name: user_name.to_string(),
        }
    }

    /// Decode Facebook's mojibake encoding.
    /// Facebook exports encode strings as latin-1 bytes stuffed into JSON UTF-8.
    fn decode_fb_string(s: &str) -> String {
        // Try to decode latin-1 → UTF-8 mojibake
        let bytes: Vec<u8> = s.chars().map(|c| c as u8).collect();
        String::from_utf8(bytes).unwrap_or_else(|_| s.to_string())
    }

    /// Parse a single message_*.json file.
    fn parse_message_file(
        &self,
        path: &Path,
        thread_name: &str,
    ) -> Result<Vec<CommonRecord>, String> {
        let data = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

        let json: serde_json::Value = serde_json::from_str(&data)
            .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;

        let messages = json
            .get("messages")
            .and_then(|m| m.as_array())
            .ok_or_else(|| format!("No messages array in {}", path.display()))?;

        let mut records = Vec::new();

        for msg in messages {
            let sender = msg
                .get("sender_name")
                .and_then(|s| s.as_str())
                .map(Self::decode_fb_string)
                .unwrap_or_default();

            let content = msg
                .get("content")
                .and_then(|s| s.as_str())
                .map(Self::decode_fb_string)
                .unwrap_or_default();

            // Skip empty messages (photo-only, reaction-only, etc.)
            if content.is_empty() {
                continue;
            }

            let timestamp_ms = msg
                .get("timestamp_ms")
                .and_then(|t| t.as_u64())
                .unwrap_or(0);

            // Convert Facebook timestamp (Unix ms) to ISO 8601
            let timestamp = if timestamp_ms > 0 {
                let secs = timestamp_ms / 1000;
                // Basic conversion — good enough for sorting
                Some(format_unix_timestamp(secs))
            } else {
                None
            };

            let is_user = sender.contains(&self.user_name) || self.user_name.contains(&sender);

            let content_hash = CommonRecord::compute_content_hash(&content);

            records.push(CommonRecord {
                content,
                timestamp,
                actor: Some(sender),
                is_user,
                source_file: path.to_string_lossy().to_string(),
                source_type: "facebook_takeout_raw".to_string(),
                trust_level: TrustLevel::Primary,
                content_hash,
                platform: "facebook".to_string(),
                thread_id: Some(thread_name.to_string()),
                thread_name: Some(Self::decode_fb_string(thread_name)),
                account: Some(AccountContext {
                    platform: "facebook".to_string(),
                    account_id: self.user_name.clone(),
                    display_name: self.user_name.clone(),
                    account_type: "personal".to_string(),
                    persona_notes: None,
                }),
                metadata: serde_json::json!({
                    "has_photos": msg.get("photos").is_some(),
                    "has_reactions": msg.get("reactions").is_some(),
                }),
            });
        }

        Ok(records)
    }
}

impl super::SourceAdapter for FacebookAdapter {
    fn name(&self) -> &str {
        "Facebook"
    }

    fn platform(&self) -> &str {
        "facebook"
    }

    fn can_handle_file(&self, path: &Path) -> bool {
        // Facebook Takeout has your_facebook_activity/messages/ structure
        path.join("your_facebook_activity")
            .join("messages")
            .exists()
    }

    fn extract_from_file(&self, path: &Path) -> Result<Vec<CommonRecord>, String> {
        let messages_dir = path.join("your_facebook_activity").join("messages");

        if !messages_dir.exists() {
            return Err(format!(
                "No messages directory at {}",
                messages_dir.display()
            ));
        }

        let mut all_records = Vec::new();

        // Scan both inbox/ and archived_threads/
        for folder in &[
            "inbox",
            "archived_threads",
            "filtered_threads",
            "message_requests",
        ] {
            let folder_path = messages_dir.join(folder);
            if !folder_path.exists() {
                continue;
            }

            // Each subdirectory is a thread
            let threads = std::fs::read_dir(&folder_path)
                .map_err(|e| format!("Failed to read {}: {}", folder_path.display(), e))?;

            for thread_entry in threads.flatten() {
                let thread_path = thread_entry.path();
                if !thread_path.is_dir() {
                    continue;
                }

                let thread_name = thread_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                // Each message_*.json in the thread
                let files = std::fs::read_dir(&thread_path).map_err(|e| {
                    format!("Failed to read thread {}: {}", thread_path.display(), e)
                })?;

                for file_entry in files.flatten() {
                    let file_path = file_entry.path();
                    if file_path.extension().and_then(|e| e.to_str()) != Some("json") {
                        continue;
                    }
                    if !file_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .starts_with("message_")
                    {
                        continue;
                    }

                    match self.parse_message_file(&file_path, &thread_name) {
                        Ok(records) => {
                            all_records.extend(records);
                        }
                        Err(e) => {
                            warn!(file = %file_path.display(), error = %e, "failed to parse message file");
                        }
                    }
                }
            }
        }

        info!(
            records = all_records.len(),
            "Facebook Takeout extraction complete"
        );
        Ok(all_records)
    }

    fn discover_local(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();

        // Common locations for Facebook Takeout
        let candidates = [
            // Nick's known locations
            PathBuf::from("D:/EVA/SUBSTRATE/data/raw/facebook_full"),
            PathBuf::from("D:/EVA/SUBSTRATE/data/raw/facebook"),
            PathBuf::from("D:/EVA/SUBSTRATE/data/raw/facebook2"),
            PathBuf::from("D:/EVA/SUBSTRATE/data/raw/facebook3"),
            // Common download locations
            dirs_next::download_dir()
                .unwrap_or_default()
                .join("facebook-data"),
            dirs_next::home_dir().unwrap_or_default().join("Downloads"),
        ];

        for path in &candidates {
            if self.can_handle_file(path) {
                found.push(path.clone());
            }
        }

        found
    }

    fn supports_api(&self) -> bool {
        // Facebook Graph API exists but is heavily restricted
        // File import is the primary path for historical data
        false
    }
}

