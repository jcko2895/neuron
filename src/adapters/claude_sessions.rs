//! Claude Sessions adapter — reads JSONL conversation logs from Claude Code.
//!
//! Claude Code stores session transcripts as JSONL files under:
//!   ~/.claude/projects/**/*.jsonl
//!
//! Each line is a JSON object with at minimum:
//!   {"type": "user"|"assistant", "message": {"content": "..."}, "timestamp": "...", "sessionId": "..."}
//!
//! We skip lines that aren't type "user" or "assistant" (tool results, meta entries, etc).

use crate::common::{AccountContext, CommonRecord, TrustLevel};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

pub struct ClaudeSessionsAdapter;

impl ClaudeSessionsAdapter {
    pub fn new() -> Self {
        Self
    }

    /// Parse a single JSONL file into records.
    fn parse_jsonl(&self, path: &Path) -> Result<Vec<CommonRecord>, String> {
        let data = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

        let mut records = Vec::new();

        for (line_num, line) in data.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let json: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Only process user and assistant messages
            let msg_type = match json.get("type").and_then(|t| t.as_str()) {
                Some("user") | Some("assistant") => {
                    json.get("type").and_then(|t| t.as_str()).unwrap()
                }
                _ => continue,
            };

            // Extract content — handle both string and structured content
            let content = if let Some(msg) = json.get("message") {
                if let Some(content) = msg.get("content") {
                    if let Some(s) = content.as_str() {
                        s.to_string()
                    } else if let Some(arr) = content.as_array() {
                        // Content can be an array of blocks: [{"type": "text", "text": "..."}]
                        let texts: Vec<&str> = arr
                            .iter()
                            .filter_map(|block| {
                                if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                                    block.get("text").and_then(|t| t.as_str())
                                } else if block.get("type").and_then(|t| t.as_str())
                                    == Some("tool_result")
                                {
                                    // Skip tool results
                                    None
                                } else if block.get("type").and_then(|t| t.as_str())
                                    == Some("tool_use")
                                {
                                    // Skip tool use blocks
                                    None
                                } else {
                                    None
                                }
                            })
                            .collect();
                        if texts.is_empty() {
                            continue;
                        }
                        texts.join("\n")
                    } else {
                        continue;
                    }
                } else {
                    continue;
                }
            } else {
                continue;
            };

            if content.trim().is_empty() {
                continue;
            }

            let timestamp = json
                .get("timestamp")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string());

            let session_id = json
                .get("sessionId")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string());

            let is_user = msg_type == "user";

            // Truncate very long messages
            let content = if content.len() > 4000 {
                let mut end = 4000;
                while end > 0 && !content.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}...", &content[..end])
            } else {
                content
            };

            let content_hash = CommonRecord::compute_content_hash(&content);

            records.push(CommonRecord {
                content,
                timestamp,
                actor: Some(if is_user { "user" } else { "claude" }.to_string()),
                is_user,
                source_file: path.to_string_lossy().to_string(),
                source_type: "claude_session_jsonl".to_string(),
                trust_level: TrustLevel::Primary,
                content_hash,
                platform: "claude".to_string(),
                thread_id: session_id.clone(),
                thread_name: session_id,
                account: Some(AccountContext {
                    platform: "claude".to_string(),
                    account_id: "nick".to_string(),
                    display_name: "Nick".to_string(),
                    account_type: "personal".to_string(),
                    persona_notes: None,
                }),
                metadata: serde_json::json!({
                    "role": msg_type,
                    "line_number": line_num + 1,
                }),
            });
        }

        Ok(records)
    }
}

impl super::SourceAdapter for ClaudeSessionsAdapter {
    fn name(&self) -> &str {
        "Claude Sessions"
    }

    fn platform(&self) -> &str {
        "claude"
    }

    fn can_handle_file(&self, path: &Path) -> bool {
        // Can handle a directory containing .jsonl files, or a single .jsonl
        if path.is_file() {
            return path.extension().and_then(|e| e.to_str()) == Some("jsonl");
        }
        if path.is_dir() {
            // Check if this looks like a Claude projects directory
            let pattern = path.join("**/*.jsonl");
            if let Ok(paths) = glob::glob(&pattern.to_string_lossy()) {
                return paths.count() > 0;
            }
        }
        false
    }

    fn extract_from_file(&self, path: &Path) -> Result<Vec<CommonRecord>, String> {
        let mut all_records = Vec::new();

        if path.is_file() {
            return self.parse_jsonl(path);
        }

        // Walk for .jsonl files
        let pattern = path.join("**/*.jsonl");
        let paths = glob::glob(&pattern.to_string_lossy())
            .map_err(|e| format!("Glob error: {}", e))?;

        for entry in paths {
            match entry {
                Ok(jsonl_path) => match self.parse_jsonl(&jsonl_path) {
                    Ok(records) => {
                        all_records.extend(records);
                    }
                    Err(e) => {
                        warn!(file = %jsonl_path.display(), error = %e, "failed to parse JSONL");
                    }
                },
                Err(e) => {
                    warn!(error = %e, "glob entry error");
                }
            }
        }

        info!(
            records = all_records.len(),
            "Claude sessions extraction complete"
        );
        Ok(all_records)
    }

    fn discover_local(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();

        let home = dirs_next::home_dir().unwrap_or_default();
        let claude_projects = home.join(".claude").join("projects");

        if claude_projects.exists() {
            found.push(claude_projects);
        }

        found
    }
}
