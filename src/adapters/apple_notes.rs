//! Apple Notes adapter — reads NoteStore.sqlite from macOS/iOS.
//!
//! Notes are stored as gzipped HTML in ZICNOTEDATA.ZDATA blob.
//! The ZICCLOUDSYNCINGOBJECT table links notes to folders/accounts.

use crate::common::{CommonRecord, TrustLevel, format_unix_timestamp};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Apple epoch offset (2001-01-01).
const APPLE_EPOCH: u64 = 978307200;

pub struct AppleNotesAdapter {
    user_name: String,
}

impl AppleNotesAdapter {
    pub fn new(user_name: &str) -> Self {
        Self { user_name: user_name.to_string() }
    }

    fn strip_html(html: &str) -> String {
        let mut result = String::new();
        let mut in_tag = false;
        for ch in html.chars() {
            match ch {
                '<' => in_tag = true,
                '>' => { in_tag = false; }
                _ if !in_tag => result.push(ch),
                _ => {}
            }
        }
        // Collapse whitespace
        let collapsed: String = result.split_whitespace().collect::<Vec<_>>().join(" ");
        collapsed.trim().to_string()
    }
}

impl super::SourceAdapter for AppleNotesAdapter {
    fn name(&self) -> &str { "Apple Notes" }
    fn platform(&self) -> &str { "apple_notes" }

    fn can_handle_file(&self, path: &Path) -> bool {
        let p = if path.is_dir() { path.join("notes.sqlite") } else { path.to_path_buf() };
        p.exists() && p.file_name().and_then(|n| n.to_str())
            .map(|n| n.contains("NoteStore") || n == "notes.sqlite")
            .unwrap_or(false)
    }

    fn extract_from_file(&self, path: &Path) -> Result<Vec<CommonRecord>, String> {
        let db_path = if path.is_dir() { path.join("notes.sqlite") } else { path.to_path_buf() };
        let conn = rusqlite::Connection::open_with_flags(
            &db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ).map_err(|e| format!("Failed to open notes db: {}", e))?;

        let mut records = Vec::new();
        // Join note data with note metadata
        let mut stmt = conn.prepare(
            "SELECT n.Z_PK, n.ZTITLE1, n.ZMODIFICATIONDATE1, n.ZCREATIONDATE1,
                    n.ZFOLDER, d.ZDATA
             FROM ZICCLOUDSYNCINGOBJECT n
             LEFT JOIN ZICNOTEDATA d ON d.ZNOTE = n.Z_PK
             WHERE n.ZTITLE1 IS NOT NULL"
        ).map_err(|e| format!("SQL failed: {}", e))?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<f64>>(2)?,
                row.get::<_, Option<f64>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<Vec<u8>>>(5)?,
            ))
        }).map_err(|e| format!("Query failed: {}", e))?;

        for row in rows {
            let (pk, title, mod_date, create_date, _folder, data_blob) = match row {
                Ok(r) => r,
                Err(_) => continue,
            };

            let title = match title {
                Some(t) if !t.is_empty() => t,
                _ => continue,
            };

            let timestamp = create_date.or(mod_date).map(|d| {
                format_unix_timestamp((d as u64) + APPLE_EPOCH)
            });

            // Try to decompress and extract text from the blob
            let body_text = data_blob.and_then(|blob| {
                // Try gzip decompression
                use std::io::Read;
                let mut decoder = flate2::read::GzDecoder::new(&blob[..]);
                let mut html = String::new();
                if decoder.read_to_string(&mut html).is_ok() {
                    let text = Self::strip_html(&html);
                    if !text.is_empty() { return Some(text); }
                }
                // Might not be gzipped — try raw
                if let Ok(text) = String::from_utf8(blob.clone()) {
                    let stripped = Self::strip_html(&text);
                    if !stripped.is_empty() { return Some(stripped); }
                }
                None
            });

            let content = if let Some(body) = &body_text {
                let preview = if body.len() > 500 {
                    let mut end = 500;
                    while !body.is_char_boundary(end) { end -= 1; }
                    format!("[Note] {} — {}...", title, &body[..end])
                } else {
                    format!("[Note] {} — {}", title, body)
                };
                preview
            } else {
                format!("[Note] {}", title)
            };

            records.push(CommonRecord {
                content: content.clone(),
                timestamp,
                actor: Some(self.user_name.clone()),
                is_user: true,
                source_file: db_path.to_string_lossy().to_string(),
                source_type: "apple_notes".into(),
                trust_level: TrustLevel::Primary,
                content_hash: CommonRecord::compute_content_hash(&content),
                platform: "apple_notes".into(),
                thread_id: Some(format!("note_{}", pk)),
                thread_name: Some(title),
                account: None,
                metadata: serde_json::json!({}),
            });
        }

        info!(records = records.len(), "Apple Notes extraction complete");
        Ok(records)
    }

    fn discover_local(&self) -> Vec<PathBuf> {
        vec![PathBuf::from("D:/EVA/SUBSTRATE/data/raw/macbook/notes.sqlite")]
            .into_iter().filter(|p| p.exists()).collect()
    }
}
