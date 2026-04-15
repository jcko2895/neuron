//! Snapchat adapter — parses Snapchat's "My Data" export.
//!
//! Export structure (across multiple zip files):
//! ```
//! json/chat_history.json  — all messages keyed by username
//! json/friends.json       — username → display name mapping
//! json/memories_history.json
//! json/location_history.json
//! json/account.json
//! chat_media/             — photos/videos from chats
//! memories/               — saved snaps
//! ```

use crate::common::{CommonRecord, TrustLevel};
use std::path::{Path, PathBuf};
use tracing::info;

pub struct SnapchatAdapter {
    user_name: String,
    user_snap_name: String,
}

impl SnapchatAdapter {
    pub fn new(user_name: &str, snap_username: &str) -> Self {
        Self {
            user_name: user_name.to_string(),
            user_snap_name: snap_username.to_string(),
        }
    }
}

impl super::SourceAdapter for SnapchatAdapter {
    fn name(&self) -> &str { "Snapchat" }
    fn platform(&self) -> &str { "snapchat" }

    fn can_handle_file(&self, path: &Path) -> bool {
        // Zip file with mydata~ prefix
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("zip") {
            return path.file_name().and_then(|n| n.to_str())
                .map(|n| n.starts_with("mydata~"))
                .unwrap_or(false);
        }
        // Directory containing extracted json/chat_history.json
        if path.is_dir() {
            return path.join("json").join("chat_history.json").exists();
        }
        false
    }

    fn extract_from_file(&self, path: &Path) -> Result<Vec<CommonRecord>, String> {
        // Handle both zip and extracted directory
        let (chat_data, friends_data) = if path.is_file() {
            let file = std::fs::File::open(path)
                .map_err(|e| format!("Failed to open zip: {}", e))?;
            let mut archive = zip::ZipArchive::new(file)
                .map_err(|e| format!("Failed to read zip: {}", e))?;

            let chat = {
                let mut f = archive.by_name("json/chat_history.json")
                    .map_err(|e| format!("chat_history.json not found: {}", e))?;
                let mut s = String::new();
                std::io::Read::read_to_string(&mut f, &mut s)
                    .map_err(|e| format!("Failed to read chat_history: {}", e))?;
                s
            };

            let friends = {
                match archive.by_name("json/friends.json") {
                    Ok(mut f) => {
                        let mut s = String::new();
                        std::io::Read::read_to_string(&mut f, &mut s).ok();
                        s
                    }
                    Err(_) => String::from("{}"),
                }
            };

            (chat, friends)
        } else {
            let chat = std::fs::read_to_string(path.join("json").join("chat_history.json"))
                .map_err(|e| format!("Failed to read chat_history: {}", e))?;
            let friends = std::fs::read_to_string(path.join("json").join("friends.json"))
                .unwrap_or_else(|_| "{}".to_string());
            (chat, friends)
        };

        // Build username → display name map
        let mut name_map = std::collections::HashMap::new();
        if let Ok(friends_json) = serde_json::from_str::<serde_json::Value>(&friends_data) {
            if let Some(friends_list) = friends_json.get("Friends").and_then(|v| v.as_array()) {
                for f in friends_list {
                    let username = f.get("Username").and_then(|v| v.as_str()).unwrap_or("");
                    let display = f.get("Display Name").and_then(|v| v.as_str()).unwrap_or("");
                    if !username.is_empty() && !display.is_empty() {
                        name_map.insert(username.to_string(), display.to_string());
                    }
                }
            }
        }

        let chats: serde_json::Value = serde_json::from_str(&chat_data)
            .map_err(|e| format!("Failed to parse chat_history: {}", e))?;

        let mut records = Vec::new();
        let source_file = path.to_string_lossy().to_string();

        let chats_obj = match chats.as_object() {
            Some(o) => o,
            None => return Ok(records),
        };

        for (username, messages) in chats_obj {
            let display_name = name_map.get(username)
                .cloned()
                .unwrap_or_else(|| username.clone());

            let messages = match messages.as_array() {
                Some(a) => a,
                None => continue,
            };

            for msg in messages {
                let content = msg.get("Content").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let media_type = msg.get("Media Type").and_then(|v| v.as_str()).unwrap_or("");
                let from = msg.get("From").and_then(|v| v.as_str()).unwrap_or("");
                let created = msg.get("Created").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let is_sender = msg.get("IsSender").and_then(|v| v.as_bool()).unwrap_or(false);
                let is_saved = msg.get("IsSaved").and_then(|v| v.as_bool()).unwrap_or(false);

                // Skip empty snaps (STATUS/MEDIA with no text)
                let display_content = if content.is_empty() {
                    match media_type {
                        "STATUS" => continue, // Skip status updates with no content
                        "MEDIA" => format!("[Snap {} sent media]", if is_sender { "you" } else { &display_name }),
                        _ => continue,
                    }
                } else {
                    content.clone()
                };

                let timestamp = if !created.is_empty() {
                    // Format: "2026-03-30 21:19:12 UTC"
                    Some(created.replace(" UTC", "+00:00").replace(' ', "T"))
                } else {
                    None
                };

                let actor = if is_sender {
                    Some(self.user_name.clone())
                } else {
                    let resolved = name_map.get(from).cloned().unwrap_or_else(|| from.to_string());
                    Some(resolved)
                };

                records.push(CommonRecord {
                    content: display_content,
                    timestamp,
                    actor,
                    is_user: is_sender,
                    source_file: source_file.clone(),
                    source_type: "snapchat_chat_history".into(),
                    trust_level: TrustLevel::Primary,
                    content_hash: CommonRecord::compute_content_hash(&content),
                    platform: "snapchat".into(),
                    thread_id: Some(username.clone()),
                    thread_name: Some(display_name.clone()),
                    account: None,
                    metadata: serde_json::json!({
                        "username": username,
                        "display_name": display_name,
                        "media_type": media_type,
                        "is_saved": is_saved,
                        "from_username": from,
                    }),
                });
            }
        }

        info!(records = records.len(), "Snapchat extraction complete");
        Ok(records)
    }

    fn discover_local(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        if let Some(downloads) = dirs_next::download_dir() {
            if let Ok(entries) = std::fs::read_dir(&downloads) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if name.starts_with("mydata~") && name.ends_with(".zip") {
                        found.push(entry.path());
                    }
                }
            }
        }
        found
    }
}
