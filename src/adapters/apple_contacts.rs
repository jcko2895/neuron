//! Apple Contacts adapter — reads AddressBook-v22.abcddb from macOS.

use crate::common::{CommonRecord, TrustLevel};
use std::path::{Path, PathBuf};
use tracing::info;

pub struct AppleContactsAdapter {
    user_name: String,
}

impl AppleContactsAdapter {
    pub fn new(user_name: &str) -> Self {
        Self { user_name: user_name.to_string() }
    }
}

impl super::SourceAdapter for AppleContactsAdapter {
    fn name(&self) -> &str { "Apple Contacts" }
    fn platform(&self) -> &str { "apple_contacts" }

    fn can_handle_file(&self, path: &Path) -> bool {
        if path.is_dir() {
            // Scan for .abcddb files
            if let Ok(entries) = std::fs::read_dir(path) {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.extension().and_then(|x| x.to_str()) == Some("abcddb") { return true; }
                    if p.is_dir() { if self.can_handle_file(&p) { return true; } }
                }
            }
            return false;
        }
        path.extension().and_then(|x| x.to_str()) == Some("abcddb")
    }

    fn extract_from_file(&self, path: &Path) -> Result<Vec<CommonRecord>, String> {
        let mut records = Vec::new();
        let mut db_files = Vec::new();

        if path.is_dir() {
            fn find_dbs(dir: &Path, out: &mut Vec<PathBuf>) {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for e in entries.flatten() {
                        let p = e.path();
                        if p.extension().and_then(|x| x.to_str()) == Some("abcddb") { out.push(p); }
                        else if p.is_dir() { find_dbs(&p, out); }
                    }
                }
            }
            find_dbs(path, &mut db_files);
        } else {
            db_files.push(path.to_path_buf());
        }

        let mut seen_names = std::collections::HashSet::new();

        for db_path in &db_files {
            let conn = match rusqlite::Connection::open_with_flags(
                db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            ) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let mut stmt = match conn.prepare(
                "SELECT r.ZFIRSTNAME, r.ZLASTNAME, r.ZORGANIZATION, r.ZJOBTITLE,
                        r.ZNICKNAME, r.Z_PK,
                        e.ZADDRESS as email,
                        p.ZFULLNUMBER as phone
                 FROM ZABCDRECORD r
                 LEFT JOIN ZABCDEMAILADDRESS e ON e.ZOWNER = r.Z_PK
                 LEFT JOIN ZABCDPHONENUMBER p ON p.ZOWNER = r.Z_PK
                 WHERE r.ZFIRSTNAME IS NOT NULL OR r.ZLASTNAME IS NOT NULL OR r.ZORGANIZATION IS NOT NULL"
            ) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let rows = match stmt.query_map([], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            }) {
                Ok(r) => r,
                Err(_) => continue,
            };

            for row in rows {
                let (first, last, org, job, nick, _pk, email, phone) = match row {
                    Ok(r) => r,
                    Err(_) => continue,
                };

                let name = match (first.as_deref(), last.as_deref()) {
                    (Some(f), Some(l)) => format!("{} {}", f, l),
                    (Some(f), None) => f.to_string(),
                    (None, Some(l)) => l.to_string(),
                    (None, None) => org.clone().unwrap_or_default(),
                };
                if name.is_empty() { continue; }

                // Dedup by name (multiple db sources may overlap)
                let key = name.to_lowercase();
                if seen_names.contains(&key) { continue; }
                seen_names.insert(key);

                let mut parts = vec![format!("[Contact] {}", name)];
                if let Some(o) = &org { if !o.is_empty() && *o != name { parts.push(format!("Org: {}", o)); } }
                if let Some(j) = &job { if !j.is_empty() { parts.push(format!("Title: {}", j)); } }
                if let Some(e) = &email { if !e.is_empty() { parts.push(format!("Email: {}", e)); } }
                if let Some(p) = &phone { if !p.is_empty() { parts.push(format!("Phone: {}", p)); } }
                let content = parts.join(" | ");

                records.push(CommonRecord {
                    content: content.clone(),
                    timestamp: None,
                    actor: Some(name),
                    is_user: false,
                    source_file: db_path.to_string_lossy().to_string(),
                    source_type: "apple_contacts".into(),
                    trust_level: TrustLevel::Primary,
                    content_hash: CommonRecord::compute_content_hash(&content),
                    platform: "apple_contacts".into(),
                    thread_id: None,
                    thread_name: None,
                    account: None,
                    metadata: serde_json::json!({
                        "org": org, "job_title": job, "nickname": nick,
                        "email": email, "phone": phone,
                    }),
                });
            }
        }

        info!(records = records.len(), "Apple Contacts extraction complete");
        Ok(records)
    }

    fn discover_local(&self) -> Vec<PathBuf> {
        vec![PathBuf::from("D:/EVA/SUBSTRATE/data/raw/macbook/contacts")]
            .into_iter().filter(|p| p.exists()).collect()
    }
}
