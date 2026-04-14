//! Facebook Friends adapter — parses your_friends.json from Facebook "Download Your Information".
//!
//! Data location:
//!   D:/EVA/SUBSTRATE/data/raw/facebook3/connections/friends/your_friends.json
//!
//! Format:
//! ```json
//! {
//!   "friends_v2": [
//!     {"name": "Person Name", "timestamp": 1234567890},
//!     ...
//!   ]
//! }
//! ```
//!
//! Each friend becomes a CommonRecord with the add-date as timestamp.

use crate::common::{format_unix_timestamp, AccountContext, CommonRecord, TrustLevel};
use std::path::{Path, PathBuf};
use tracing::info;

pub struct FacebookFriendsAdapter {
    user_name: String,
}

impl FacebookFriendsAdapter {
    pub fn new(user_name: &str) -> Self {
        Self {
            user_name: user_name.to_string(),
        }
    }

    /// Decode Facebook's mojibake encoding (latin-1 bytes stuffed into JSON UTF-8).
    fn decode_fb_string(s: &str) -> String {
        let bytes: Vec<u8> = s.chars().map(|c| c as u8).collect();
        String::from_utf8(bytes).unwrap_or_else(|_| s.to_string())
    }
}

impl super::SourceAdapter for FacebookFriendsAdapter {
    fn name(&self) -> &str {
        "Facebook Friends"
    }

    fn platform(&self) -> &str {
        "facebook"
    }

    fn can_handle_file(&self, path: &Path) -> bool {
        // Accept either the your_friends.json file directly, or a directory containing it
        if path.is_file() {
            return path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n == "your_friends.json")
                .unwrap_or(false);
        }
        if path.is_dir() {
            // Check standard Facebook export structure
            return path
                .join("connections")
                .join("friends")
                .join("your_friends.json")
                .exists();
        }
        false
    }

    fn extract_from_file(&self, path: &Path) -> Result<Vec<CommonRecord>, String> {
        let json_path = if path.is_file() {
            path.to_path_buf()
        } else {
            path.join("connections")
                .join("friends")
                .join("your_friends.json")
        };

        let data = std::fs::read_to_string(&json_path)
            .map_err(|e| format!("Failed to read {}: {}", json_path.display(), e))?;

        let json: serde_json::Value = serde_json::from_str(&data)
            .map_err(|e| format!("Failed to parse {}: {}", json_path.display(), e))?;

        let friends = json
            .get("friends_v2")
            .and_then(|f| f.as_array())
            .ok_or_else(|| format!("No friends_v2 array in {}", json_path.display()))?;

        let mut records = Vec::new();

        for friend in friends {
            let name_raw = friend
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("Unknown");
            let name = Self::decode_fb_string(name_raw);

            let timestamp_secs = friend
                .get("timestamp")
                .and_then(|t| t.as_u64())
                .unwrap_or(0);

            let timestamp = if timestamp_secs > 0 {
                Some(format_unix_timestamp(timestamp_secs))
            } else {
                None
            };

            let content = format!("Became friends with {}", name);
            let content_hash = CommonRecord::compute_content_hash(&content);

            records.push(CommonRecord {
                content,
                timestamp,
                actor: Some(name.clone()),
                is_user: false,
                source_file: json_path.to_string_lossy().to_string(),
                source_type: "facebook_friends_json".to_string(),
                trust_level: TrustLevel::Primary,
                content_hash,
                platform: "facebook".to_string(),
                thread_id: None,
                thread_name: None,
                account: Some(AccountContext {
                    platform: "facebook".to_string(),
                    account_id: self.user_name.clone(),
                    display_name: self.user_name.clone(),
                    account_type: "personal".to_string(),
                    persona_notes: None,
                }),
                metadata: serde_json::json!({
                    "friend_name": name,
                    "added_timestamp": timestamp_secs,
                }),
            });
        }

        info!(
            records = records.len(),
            "Facebook friends extraction complete"
        );
        Ok(records)
    }

    fn discover_local(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();

        let candidates = [
            PathBuf::from("D:/EVA/SUBSTRATE/data/raw/facebook3"),
            PathBuf::from("D:/EVA/SUBSTRATE/data/raw/facebook_full"),
            PathBuf::from("D:/EVA/SUBSTRATE/data/raw/facebook2"),
            PathBuf::from("D:/EVA/SUBSTRATE/data/raw/facebook"),
        ];

        for path in &candidates {
            if self.can_handle_file(path) {
                found.push(path.clone());
            }
        }

        found
    }
}
