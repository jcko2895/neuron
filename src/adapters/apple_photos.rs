//! Apple Photos metadata adapter — reads Photos.sqlite from macOS/iOS.
//!
//! Extracts photo metadata: dates, GPS coordinates, filenames, dimensions.
//! Does NOT copy actual image files — just the metadata for timeline/location building.

use crate::common::{CommonRecord, TrustLevel, format_unix_timestamp};
use std::path::{Path, PathBuf};
use tracing::info;

const APPLE_EPOCH: u64 = 978307200;

pub struct ApplePhotosAdapter {
    user_name: String,
}

impl ApplePhotosAdapter {
    pub fn new(user_name: &str) -> Self {
        Self { user_name: user_name.to_string() }
    }
}

impl super::SourceAdapter for ApplePhotosAdapter {
    fn name(&self) -> &str { "Apple Photos" }
    fn platform(&self) -> &str { "apple_photos" }

    fn can_handle_file(&self, path: &Path) -> bool {
        let p = if path.is_dir() { path.join("photos.sqlite") } else { path.to_path_buf() };
        p.exists() && p.file_name().and_then(|n| n.to_str())
            .map(|n| n == "Photos.sqlite" || n == "photos.sqlite")
            .unwrap_or(false)
    }

    fn extract_from_file(&self, path: &Path) -> Result<Vec<CommonRecord>, String> {
        let db_path = if path.is_dir() { path.join("photos.sqlite") } else { path.to_path_buf() };
        let conn = rusqlite::Connection::open_with_flags(
            &db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ).map_err(|e| format!("Failed to open Photos.sqlite: {}", e))?;

        let mut records = Vec::new();

        let mut stmt = conn.prepare(
            "SELECT ZDATECREATED, ZLATITUDE, ZLONGITUDE, ZFILENAME, ZWIDTH, ZHEIGHT,
                    ZKIND, ZTRASHEDSTATE
             FROM ZASSET
             WHERE ZDATECREATED IS NOT NULL AND ZTRASHEDSTATE = 0
             ORDER BY ZDATECREATED ASC"
        ).map_err(|e| format!("SQL failed: {}", e))?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, f64>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<i32>>(4)?,
                row.get::<_, Option<i32>>(5)?,
                row.get::<_, Option<i32>>(6)?,
                row.get::<_, i32>(7)?,
            ))
        }).map_err(|e| format!("Query failed: {}", e))?;

        for row in rows {
            let (date, lat, lon, filename, width, height, kind, _trashed) = match row {
                Ok(r) => r,
                Err(_) => continue,
            };

            let unix_secs = (date as u64) + APPLE_EPOCH;
            let timestamp = Some(format_unix_timestamp(unix_secs));

            let filename = filename.unwrap_or_else(|| "unknown".to_string());
            let width = width.unwrap_or(0);
            let height = height.unwrap_or(0);

            let has_gps = lat > -180.0 && lat < 180.0 && lon > -180.0 && lon < 180.0
                         && lat != 0.0 && lon != 0.0;

            let media_type = match kind {
                Some(0) => "Photo",
                Some(1) => "Video",
                _ => "Media",
            };

            let content = if has_gps {
                format!("[{} {}x{}] {} @ ({:.4}, {:.4})", media_type, width, height, filename, lat, lon)
            } else {
                format!("[{} {}x{}] {}", media_type, width, height, filename)
            };

            records.push(CommonRecord {
                content: content.clone(),
                timestamp,
                actor: Some(self.user_name.clone()),
                is_user: true,
                source_file: db_path.to_string_lossy().to_string(),
                source_type: "apple_photos_metadata".into(),
                trust_level: TrustLevel::Primary,
                content_hash: CommonRecord::compute_content_hash(&content),
                platform: "apple_photos".into(),
                thread_id: None,
                thread_name: None,
                account: None,
                metadata: serde_json::json!({
                    "filename": filename,
                    "width": width,
                    "height": height,
                    "latitude": if has_gps { Some(lat) } else { None },
                    "longitude": if has_gps { Some(lon) } else { None },
                    "media_type": media_type,
                }),
            });
        }

        info!(records = records.len(), "Apple Photos metadata extraction complete");
        Ok(records)
    }

    fn discover_local(&self) -> Vec<PathBuf> {
        vec![PathBuf::from("D:/EVA/SUBSTRATE/data/raw/macbook/photos.sqlite")]
            .into_iter().filter(|p| p.exists()).collect()
    }
}
