//! ChatGPT adapter — parses OpenAI's "Export your data" archive.
//!
//! Export structure:
//! ```
//! ChatGPT_Export-YYYY-MM-DD.zip
//!   conversations.json    — all conversations with full message trees
//!   user.json             — account info
//!   message_feedback.json — thumbs up/down on responses
//!   shared_conversations.json
//!   *.png, *.jpeg, *.wav  — attachments (images, audio)
//! ```
//!
//! conversations.json is an array of conversation objects, each containing
//! a `mapping` dict of message nodes with parent/child relationships.

use crate::common::{CommonRecord, TrustLevel, format_unix_timestamp};
use std::path::{Path, PathBuf};
use tracing::info;

pub struct ChatGptAdapter {
    user_name: String,
}

impl ChatGptAdapter {
    pub fn new(user_name: &str) -> Self {
        Self { user_name: user_name.to_string() }
    }
}

impl super::SourceAdapter for ChatGptAdapter {
    fn name(&self) -> &str { "ChatGPT" }
    fn platform(&self) -> &str { "chatgpt" }

    fn can_handle_file(&self, path: &Path) -> bool {
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("zip") {
            return path.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("ChatGPT_Export"))
                .unwrap_or(false);
        }
        // Extracted directory with conversations.json
        if path.is_dir() {
            return path.join("conversations.json").exists();
        }
        // Direct conversations.json file
        if path.is_file() {
            return path.file_name().and_then(|n| n.to_str()) == Some("conversations.json");
        }
        false
    }

    fn extract_from_file(&self, path: &Path) -> Result<Vec<CommonRecord>, String> {
        let conversations_data = if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("zip") {
            // Read from zip
            let file = std::fs::File::open(path)
                .map_err(|e| format!("Failed to open zip: {}", e))?;
            let mut archive = zip::ZipArchive::new(file)
                .map_err(|e| format!("Failed to read zip: {}", e))?;
            let mut conv_file = archive.by_name("conversations.json")
                .map_err(|e| format!("conversations.json not found in zip: {}", e))?;
            let mut data = String::new();
            std::io::Read::read_to_string(&mut conv_file, &mut data)
                .map_err(|e| format!("Failed to read conversations.json: {}", e))?;
            data
        } else if path.is_dir() {
            std::fs::read_to_string(path.join("conversations.json"))
                .map_err(|e| format!("Failed to read conversations.json: {}", e))?
        } else {
            std::fs::read_to_string(path)
                .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?
        };

        let conversations: Vec<serde_json::Value> = serde_json::from_str(&conversations_data)
            .map_err(|e| format!("Failed to parse conversations.json: {}", e))?;

        let mut records = Vec::new();
        let source_file = path.to_string_lossy().to_string();

        for conv in &conversations {
            let title = conv.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled").to_string();
            let conv_id = conv.get("conversation_id")
                .or_else(|| conv.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let mapping = match conv.get("mapping").and_then(|v| v.as_object()) {
                Some(m) => m,
                None => continue,
            };

            for (_node_id, node) in mapping {
                let message = match node.get("message") {
                    Some(m) if !m.is_null() => m,
                    _ => continue,
                };

                let role = message.get("author").and_then(|a| a.get("role")).and_then(|r| r.as_str()).unwrap_or("");
                // Only ingest user and assistant messages, skip system
                if role != "user" && role != "assistant" { continue; }

                // Extract text content from the parts array
                let content_parts = message.get("content")
                    .and_then(|c| c.get("parts"))
                    .and_then(|p| p.as_array());

                let content = match content_parts {
                    Some(parts) => {
                        let texts: Vec<String> = parts.iter()
                            .filter_map(|p| p.as_str().map(|s| s.to_string()))
                            .collect();
                        texts.join("\n")
                    }
                    None => continue,
                };

                if content.is_empty() { continue; }

                // Timestamp
                let create_time = message.get("create_time").and_then(|t| t.as_f64()).unwrap_or(0.0);
                let timestamp = if create_time > 0.0 {
                    Some(format_unix_timestamp(create_time as u64))
                } else {
                    None
                };

                // Model info (for assistant messages)
                let model = message.get("metadata")
                    .and_then(|m| m.get("model_slug"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let is_user = role == "user";
                let actor = if is_user {
                    Some(self.user_name.clone())
                } else {
                    Some(format!("ChatGPT ({})", if model.is_empty() { "unknown" } else { &model }))
                };

                // Truncate very long assistant responses for the content field
                let display_content = if content.len() > 2000 {
                    let mut end = 2000;
                    while !content.is_char_boundary(end) { end -= 1; }
                    format!("{}...", &content[..end])
                } else {
                    content.clone()
                };

                let prefixed = format!("[ChatGPT: {}] [{}] {}", title, role, display_content);

                records.push(CommonRecord {
                    content: prefixed.clone(),
                    timestamp,
                    actor,
                    is_user,
                    source_file: source_file.clone(),
                    source_type: "chatgpt_export".into(),
                    trust_level: if is_user { TrustLevel::Primary } else { TrustLevel::Secondary },
                    content_hash: CommonRecord::compute_content_hash(&prefixed),
                    platform: "chatgpt".into(),
                    thread_id: Some(conv_id.clone()),
                    thread_name: Some(title.clone()),
                    account: None,
                    metadata: serde_json::json!({
                        "role": role,
                        "model": model,
                        "conversation_title": title,
                    }),
                });
            }
        }

        info!(records = records.len(), conversations = conversations.len(), "ChatGPT extraction complete");
        Ok(records)
    }

    fn discover_local(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let candidates = [
            PathBuf::from("G:/archive/downloads/ChatGPT_Export-2025-09-06.zip"),
        ];
        if let Some(downloads) = dirs_next::download_dir() {
            if let Ok(entries) = std::fs::read_dir(&downloads) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if name.starts_with("ChatGPT_Export") && name.ends_with(".zip") {
                        found.push(entry.path());
                    }
                }
            }
        }
        for p in &candidates {
            if p.exists() { found.push(p.clone()); }
        }
        found
    }
}
