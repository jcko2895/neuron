//! iMessage SQLite adapter — reads Apple's chat.db directly.
//!
//! This is the native iMessage database from macOS/iOS.
//! Contains full message history synced via iCloud.
//!
//! Schema:
//! - message: text, date (nanoseconds since 2001-01-01), is_from_me, handle_id, service
//! - handle: id (phone/email), service
//! - chat: display_name, chat_identifier (group chats)
//! - chat_message_join: links messages to chats
//!
//! Apple epoch: 2001-01-01 00:00:00 UTC = Unix 978307200

use crate::common::{CommonRecord, TrustLevel, format_unix_timestamp};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Seconds between Unix epoch (1970) and Apple epoch (2001).
const APPLE_EPOCH_OFFSET: u64 = 978307200;

pub struct IMessageDbAdapter {
    user_name: String,
}

impl IMessageDbAdapter {
    pub fn new(user_name: &str) -> Self {
        Self { user_name: user_name.to_string() }
    }

    /// Extract plain text from Apple's NSAttributedString streamtyped blob.
    ///
    /// Modern iMessage stores message text in `attributedBody` as a serialized
    /// NSAttributedString, not in the `text` column. The blob format is:
    /// `streamtyped` header → class hierarchy → NSString/NSMutableString →
    /// `+` marker → length byte(s) → UTF-8 text bytes.
    fn extract_text_from_attributed_body(blob: &[u8]) -> Option<String> {
        // Find the '+' (0x2b) byte that precedes the text length+data.
        // It appears after the NSString/NSMutableString class markers.
        // Scan the entire blob for it after any string class marker.
        for marker in &[b"NSMutableString".as_slice(), b"NSString".as_slice()] {
            let idx = match blob.windows(marker.len()).position(|w| w == *marker) {
                Some(i) => i,
                None => continue,
            };
            // Find the '+' byte (0x2b) anywhere after the marker within 50 bytes
            let search_start = idx + marker.len();
            let search_end = (search_start + 50).min(blob.len());
            for offset in search_start..search_end {
                if blob[offset] != 0x2b { continue; }
                // Found '+' — next byte(s) are the string length
                if offset + 1 >= blob.len() { break; }
                let (length, text_start) = match blob[offset + 1] {
                    0x81 if offset + 3 < blob.len() => {
                        let len = ((blob[offset + 2] as usize) << 8) | (blob[offset + 3] as usize);
                        (len, offset + 4)
                    }
                    0x82 if offset + 4 < blob.len() => {
                        let len = ((blob[offset + 2] as usize) << 8) | (blob[offset + 3] as usize);
                        (len, offset + 4)
                    }
                    n => (n as usize, offset + 2),
                };
                if length == 0 || text_start + length > blob.len() { break; }
                if let Ok(text) = std::str::from_utf8(&blob[text_start..text_start + length]) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    }
                }
                break;
            }
        }
        None
    }
}

impl super::SourceAdapter for IMessageDbAdapter {
    fn name(&self) -> &str { "iMessage (chat.db)" }
    fn platform(&self) -> &str { "imessage" }

    fn can_handle_file(&self, path: &Path) -> bool {
        if path.is_file() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                return name == "chat.db";
            }
        }
        // Directory containing chat.db
        if path.is_dir() {
            return path.join("chat.db").exists();
        }
        false
    }

    fn extract_from_file(&self, path: &Path) -> Result<Vec<CommonRecord>, String> {
        let db_path = if path.is_dir() {
            path.join("chat.db")
        } else {
            path.to_path_buf()
        };

        let conn = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ).map_err(|e| format!("Failed to open chat.db: {}", e))?;

        let mut records = Vec::new();

        // Query messages with handle and chat info
        // Include both text and attributedBody — modern iMessage stores text in the blob
        let mut stmt = conn.prepare(
            "SELECT
                m.ROWID,
                m.text,
                m.attributedBody,
                m.is_from_me,
                m.date,
                m.service,
                m.cache_roomnames,
                m.group_title,
                m.associated_message_type,
                h.id as handle_id,
                c.display_name as chat_display_name,
                c.chat_identifier
            FROM message m
            LEFT JOIN handle h ON m.handle_id = h.ROWID
            LEFT JOIN chat_message_join cmj ON m.ROWID = cmj.message_id
            LEFT JOIN chat c ON cmj.chat_id = c.ROWID
            WHERE m.associated_message_type = 0
            AND (m.text IS NOT NULL OR m.attributedBody IS NOT NULL)
            ORDER BY m.date ASC"
        ).map_err(|e| format!("SQL prepare failed: {}", e))?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,           // rowid
                row.get::<_, Option<String>>(1)?, // text (may be NULL)
                row.get::<_, Option<Vec<u8>>>(2)?, // attributedBody blob
                row.get::<_, i32>(3)?,           // is_from_me
                row.get::<_, i64>(4)?,           // date
                row.get::<_, Option<String>>(5)?, // service
                row.get::<_, Option<String>>(6)?, // cache_roomnames
                row.get::<_, Option<String>>(7)?, // group_title
                row.get::<_, i32>(8)?,           // associated_message_type
                row.get::<_, Option<String>>(9)?, // handle_id
                row.get::<_, Option<String>>(10)?, // chat_display_name
                row.get::<_, Option<String>>(11)?, // chat_identifier
            ))
        }).map_err(|e| format!("SQL query failed: {}", e))?;

        for row in rows {
            let (rowid, text_col, attributed_body, is_from_me, date_ns, service,
                 cache_roomnames, group_title, _assoc_type, handle_id,
                 chat_display_name, chat_identifier) =
                match row {
                    Ok(r) => r,
                    Err(_) => continue,
                };

            // Extract text: prefer text column, fall back to attributedBody blob
            let text = text_col
                .filter(|t| !t.is_empty())
                .or_else(|| attributed_body.as_deref().and_then(Self::extract_text_from_attributed_body));

            let text = match text {
                Some(t) if !t.is_empty() => t,
                _ => continue,
            };

            // Convert Apple nanosecond timestamp to Unix seconds
            let unix_secs = if date_ns > 0 {
                (date_ns as u64 / 1_000_000_000) + APPLE_EPOCH_OFFSET
            } else {
                0
            };
            let timestamp = if unix_secs > APPLE_EPOCH_OFFSET {
                Some(format_unix_timestamp(unix_secs))
            } else {
                None
            };

            let is_user = is_from_me == 1;
            let service = service.unwrap_or_else(|| "iMessage".to_string());
            let handle = handle_id.unwrap_or_else(|| "unknown".to_string());

            let actor = if is_user {
                Some(self.user_name.clone())
            } else {
                Some(handle.clone())
            };

            // Thread identification: use group chat name, or the handle for 1:1
            let thread_name = group_title
                .or(chat_display_name)
                .or(cache_roomnames.clone())
                .or(Some(handle.clone()));

            let thread_id = chat_identifier
                .or(cache_roomnames)
                .or(Some(handle.clone()));

            records.push(CommonRecord {
                content: text.clone(),
                timestamp,
                actor,
                is_user,
                source_file: db_path.to_string_lossy().to_string(),
                source_type: "imessage_chat_db".into(),
                trust_level: TrustLevel::Primary,
                content_hash: CommonRecord::compute_content_hash(&text),
                platform: "imessage".into(),
                thread_id,
                thread_name,
                account: None,
                metadata: serde_json::json!({
                    "service": service,
                    "handle": handle,
                    "rowid": rowid,
                }),
            });
        }

        info!(records = records.len(), "iMessage chat.db extraction complete");
        Ok(records)
    }

    fn discover_local(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let candidates = [
            PathBuf::from("D:/EVA/SUBSTRATE/data/raw/macbook/chat.db"),
        ];
        for p in &candidates {
            if p.exists() { found.push(p.clone()); }
        }
        found
    }
}
