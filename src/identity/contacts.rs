//! Apple/Google contacts database reader.
//!
//! Reads AddressBook-v22.abcddb (macOS) to build phone→name and email→name mappings.
//! These mappings feed into the identity resolver.

use std::collections::HashMap;
use std::path::Path;
use tracing::{info, warn};

/// Phone number (last 10 digits) → full name.
pub type PhoneMap = HashMap<String, String>;
/// Email (lowercase) → full name.
pub type EmailMap = HashMap<String, String>;

/// Read all Apple Contacts databases from a directory tree.
/// Returns (phone_map, email_map).
pub fn read_apple_contacts(dir: &Path) -> (PhoneMap, EmailMap) {
    let mut phones = PhoneMap::new();
    let mut emails = EmailMap::new();

    if !dir.exists() {
        warn!(path = %dir.display(), "Contacts directory not found");
        return (phones, emails);
    }

    fn scan_dir(dir: &Path, phones: &mut PhoneMap, emails: &mut EmailMap) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                scan_dir(&path, phones, emails);
            } else if path.extension().and_then(|e| e.to_str()) == Some("abcddb") {
                read_single_db(&path, phones, emails);
            }
        }
    }

    scan_dir(dir, &mut phones, &mut emails);
    info!(phones = phones.len(), emails = emails.len(), "Apple Contacts loaded");
    (phones, emails)
}

fn read_single_db(path: &Path, phones: &mut PhoneMap, emails: &mut EmailMap) {
    let conn = match rusqlite::Connection::open_with_flags(
        path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) {
        Ok(c) => c,
        Err(_) => return,
    };

    // Phone numbers
    if let Ok(mut stmt) = conn.prepare(
        "SELECT r.ZFIRSTNAME, r.ZLASTNAME, p.ZFULLNUMBER
         FROM ZABCDRECORD r
         JOIN ZABCDPHONENUMBER p ON p.ZOWNER = r.Z_PK
         WHERE (r.ZFIRSTNAME IS NOT NULL OR r.ZLASTNAME IS NOT NULL)
         AND p.ZFULLNUMBER IS NOT NULL"
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            let first: Option<String> = row.get(0)?;
            let last: Option<String> = row.get(1)?;
            let phone: String = row.get(2)?;
            Ok((first, last, phone))
        }) {
            for row in rows.flatten() {
                let (first, last, phone) = row;
                let name = match (first.as_deref(), last.as_deref()) {
                    (Some(f), Some(l)) => format!("{} {}", f, l),
                    (Some(f), None) => f.to_string(),
                    (None, Some(l)) => l.to_string(),
                    (None, None) => continue,
                };
                if let Some(normalized) = crate::identity::normalize_phone(&phone) {
                    phones.insert(normalized, name.clone());
                }
            }
        }
    }

    // Email addresses
    if let Ok(mut stmt) = conn.prepare(
        "SELECT r.ZFIRSTNAME, r.ZLASTNAME, e.ZADDRESS
         FROM ZABCDRECORD r
         JOIN ZABCDEMAILADDRESS e ON e.ZOWNER = r.Z_PK
         WHERE (r.ZFIRSTNAME IS NOT NULL OR r.ZLASTNAME IS NOT NULL)
         AND e.ZADDRESS IS NOT NULL"
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            let first: Option<String> = row.get(0)?;
            let last: Option<String> = row.get(1)?;
            let email: String = row.get(2)?;
            Ok((first, last, email))
        }) {
            for row in rows.flatten() {
                let (first, last, email) = row;
                let name = match (first.as_deref(), last.as_deref()) {
                    (Some(f), Some(l)) => format!("{} {}", f, l),
                    (Some(f), None) => f.to_string(),
                    (None, Some(l)) => l.to_string(),
                    (None, None) => continue,
                };
                emails.insert(crate::identity::normalize_email(&email), name);
            }
        }
    }
}
