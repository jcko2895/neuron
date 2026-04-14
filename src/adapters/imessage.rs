//! iMessage adapter — parses iPhone backup message exports.
//!
//! Supports two formats:
//! - Pre-extracted JSONL files (from iphone_backup ingest scripts)
//! - Raw iPhone backup SQLite databases (sms.db)
//!
//! JSONL schema (pre-extracted):
//! ```json
//! {"id": "...", "source": "iphone_backup_iphone6_2015", "type": "imessage",
//!  "timestamp": "2014-09-06T21:30:53+00:00", "content": "Hi",
//!  "metadata": {"handle": "+14257360188", "is_from_me": true, "group_chat": null, "service": "iMessage"}}
//! ```

use crate::common::{CommonRecord, TrustLevel};
use std::path::{Path, PathBuf};
use tracing::info;

pub struct IMessageAdapter {
    user_name: String,
}

impl IMessageAdapter {
    pub fn new(user_name: &str) -> Self {
        Self { user_name: user_name.to_string() }
    }
}

impl super::SourceAdapter for IMessageAdapter {
    fn name(&self) -> &str { "iMessage" }
    fn platform(&self) -> &str { "imessage" }

    fn can_handle_file(&self, path: &Path) -> bool {
        // JSONL files from iPhone backup ingest
        if path.is_file() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                return name.contains("messages") && name.ends_with(".jsonl");
            }
        }
        // Directory containing JSONL message files
        if path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if name.contains("messages") && name.ends_with(".jsonl") {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn extract_from_file(&self, path: &Path) -> Result<Vec<CommonRecord>, String> {
        let mut records = Vec::new();

        let files: Vec<PathBuf> = if path.is_file() {
            vec![path.to_path_buf()]
        } else {
            // Scan directory for message JSONL files
            std::fs::read_dir(path)
                .map_err(|e| format!("Failed to read dir {}: {}", path.display(), e))?
                .flatten()
                .filter(|e| {
                    let name = e.file_name();
                    let name = name.to_string_lossy();
                    name.contains("messages") && name.ends_with(".jsonl")
                })
                .map(|e| e.path())
                .collect()
        };

        for file_path in &files {
            let data = std::fs::read_to_string(file_path)
                .map_err(|e| format!("Failed to read {}: {}", file_path.display(), e))?;

            for line in data.lines() {
                let line = line.trim();
                if line.is_empty() { continue; }

                let json: serde_json::Value = match serde_json::from_str(line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let content = json.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if content.is_empty() { continue; }

                let timestamp = json.get("timestamp").and_then(|v| v.as_str()).map(|s| s.to_string());
                let metadata = json.get("metadata").cloned().unwrap_or(serde_json::json!({}));
                let is_from_me = metadata.get("is_from_me").and_then(|v| v.as_bool()).unwrap_or(false);
                let handle = metadata.get("handle").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                let group_chat = metadata.get("group_chat").and_then(|v| v.as_str()).map(|s| s.to_string());
                let service = metadata.get("service").and_then(|v| v.as_str()).unwrap_or("iMessage").to_string();

                let actor = if is_from_me {
                    Some(self.user_name.clone())
                } else {
                    Some(handle.clone())
                };

                let source_name = file_path.file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                records.push(CommonRecord {
                    content: content.clone(),
                    timestamp,
                    actor,
                    is_user: is_from_me,
                    source_file: file_path.to_string_lossy().to_string(),
                    source_type: "iphone_backup_messages".into(),
                    trust_level: TrustLevel::Primary,
                    content_hash: CommonRecord::compute_content_hash(&content),
                    platform: "imessage".into(),
                    thread_id: Some(handle.clone()),
                    thread_name: group_chat.or(Some(handle)),
                    account: None,
                    metadata: serde_json::json!({
                        "service": service,
                        "backup_source": source_name,
                    }),
                });
            }
        }

        info!(records = records.len(), "iMessage extraction complete");
        Ok(records)
    }

    fn discover_local(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        // Known locations for iPhone backup message exports
        let base = PathBuf::from("D:/EVA/SUBSTRATE/data");
        if base.exists() {
            if let Ok(entries) = std::fs::read_dir(&base) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if name.starts_with("iphone_") && name.contains("messages") && name.ends_with(".jsonl") {
                        found.push(entry.path());
                    }
                }
            }
        }
        found
    }
}
