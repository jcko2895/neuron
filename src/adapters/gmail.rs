//! Gmail adapter — parses .eml files from Google Takeout export.
//!
//! Nick's Gmail Takeout is at:
//!   D:/UserData/Documents/Emails/Gmail/raw/Takeout/
//!   Organized: {year}/{month}/{timestamp}_{subject}.eml
//!   202,087 emails spanning 2009-2025
//!
//! Also handles raw MBOX format from Google Takeout ZIP exports.

use crate::common::{AccountContext, CommonRecord, TrustLevel};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

pub struct GmailAdapter {
    user_email: String,
    user_name: String,
}

impl GmailAdapter {
    pub fn new(user_email: &str, user_name: &str) -> Self {
        Self {
            user_email: user_email.to_string(),
            user_name: user_name.to_string(),
        }
    }

    /// Parse a single .eml file.
    fn parse_eml(&self, path: &Path) -> Result<CommonRecord, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

        // Parse headers
        let mut from = String::new();
        let mut to = String::new();
        let mut date = String::new();
        let mut subject = String::new();
        let mut in_headers = true;
        let mut body_lines = Vec::new();

        for line in raw.lines() {
            if in_headers {
                if line.is_empty() {
                    in_headers = false;
                    continue;
                }
                if let Some(val) = line.strip_prefix("From: ") {
                    from = val.to_string();
                } else if let Some(val) = line.strip_prefix("To: ") {
                    to = val.to_string();
                } else if let Some(val) = line.strip_prefix("Date: ") {
                    date = val.to_string();
                } else if let Some(val) = line.strip_prefix("Subject: ") {
                    subject = val.to_string();
                }
                // Handle folded headers (continuation lines starting with whitespace)
                if line.starts_with(' ') || line.starts_with('\t') {
                    if !from.is_empty() && to.is_empty() {
                        from.push_str(line.trim());
                    }
                }
            } else {
                body_lines.push(line);
            }
        }

        let body = body_lines.join("\n");

        // Extract plain text content from body
        // Skip HTML, MIME boundaries, base64 encoded content
        let content = extract_plain_text(&body);

        if content.trim().is_empty() && subject.is_empty() {
            return Err("Empty email".into());
        }

        // Build display content: subject + body preview
        let display_content = if content.trim().is_empty() {
            format!("[Subject: {}]", subject)
        } else if subject.is_empty() {
            content.clone()
        } else {
            format!("[{}] {}", subject, content)
        };

        // Truncate very long emails (safe UTF-8 boundary)
        let display_content = if display_content.len() > 2000 {
            let mut end = 2000;
            while end > 0 && !display_content.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}...", &display_content[..end])
        } else {
            display_content
        };

        let is_user = from.contains(&self.user_email) || from.contains(&self.user_name);

        let actor = extract_name_from_email(&from);
        let recipient = extract_name_from_email(&to);

        let content_hash = CommonRecord::compute_content_hash(&display_content);

        // Parse date to ISO 8601 (best effort)
        let timestamp = parse_email_date(&date);

        Ok(CommonRecord {
            content: display_content,
            timestamp,
            actor: Some(actor),
            is_user,
            source_file: path.to_string_lossy().to_string(),
            source_type: "gmail_takeout_eml".to_string(),
            trust_level: TrustLevel::Primary,
            content_hash,
            platform: "gmail".to_string(),
            thread_id: None,
            thread_name: Some(subject),
            account: Some(AccountContext {
                platform: "gmail".to_string(),
                account_id: self.user_email.clone(),
                display_name: self.user_name.clone(),
                account_type: "personal".to_string(),
                persona_notes: None,
            }),
            metadata: serde_json::json!({
                "to": to,
                "from": from,
            }),
        })
    }
}

impl super::SourceAdapter for GmailAdapter {
    fn name(&self) -> &str {
        "Gmail"
    }

    fn platform(&self) -> &str {
        "gmail"
    }

    fn can_handle_file(&self, path: &Path) -> bool {
        // Gmail Takeout has year directories with .eml files
        if path.is_dir() {
            // Check if it has year-named subdirectories
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.len() == 4 && name.parse::<u32>().is_ok() {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn extract_from_file(&self, path: &Path) -> Result<Vec<CommonRecord>, String> {
        let mut records = Vec::new();
        let mut errors = 0u64;

        // Walk all .eml files
        walk_eml_files(path, &mut |eml_path| match self.parse_eml(eml_path) {
            Ok(record) => records.push(record),
            Err(_) => errors += 1,
        });

        info!(
            records = records.len(),
            errors = errors,
            "Gmail extraction complete"
        );
        Ok(records)
    }

    fn discover_local(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();

        let candidates = [
            PathBuf::from("D:/UserData/Documents/Emails/Gmail/raw/Takeout"),
            dirs_next::home_dir()
                .unwrap_or_default()
                .join("Downloads")
                .join("Takeout")
                .join("Mail"),
        ];

        for path in &candidates {
            if self.can_handle_file(path) {
                found.push(path.clone());
            }
        }

        found
    }

    fn estimate_count(&self, path: &Path) -> Option<usize> {
        // Quick count of .eml files
        let mut count = 0usize;
        walk_eml_files(path, &mut |_| count += 1);
        Some(count)
    }
}

/// Walk directory tree and call callback for each .eml file
fn walk_eml_files(dir: &Path, callback: &mut impl FnMut(&Path)) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk_eml_files(&path, callback);
            } else if path.extension().and_then(|e| e.to_str()) == Some("eml") {
                callback(&path);
            }
        }
    }
}

/// Extract plain text from email body, skipping HTML and MIME encoding
fn extract_plain_text(body: &str) -> String {
    let mut result = Vec::new();
    let mut skip = false;
    let mut in_base64 = false;

    for line in body.lines() {
        // Skip MIME boundaries
        if line.starts_with("--") && line.len() > 10 {
            skip = false;
            in_base64 = false;
            continue;
        }

        // Detect content type headers within MIME parts
        if line.starts_with("Content-Type:") {
            if line.contains("text/html") {
                skip = true;
                in_base64 = false;
            } else if line.contains("text/plain") {
                skip = false;
                in_base64 = false;
            }
            continue;
        }

        if line.starts_with("Content-Transfer-Encoding:") {
            if line.contains("base64") {
                in_base64 = true;
            }
            continue;
        }

        // Skip Content-Disposition and similar headers in MIME parts
        if line.starts_with("Content-") {
            continue;
        }

        if skip || in_base64 {
            continue;
        }

        // Skip obvious HTML
        let trimmed = line.trim();
        if trimmed.starts_with('<') && trimmed.ends_with('>') {
            continue;
        }
        if trimmed.contains("<html") || trimmed.contains("<head") || trimmed.contains("<body") {
            skip = true;
            continue;
        }

        // Keep the line
        if !trimmed.is_empty() {
            result.push(trimmed);
        }
    }

    result.join("\n")
}

/// Extract human name from email header like "Nick Towne <nick@gmail.com>"
fn extract_name_from_email(header: &str) -> String {
    let header = header.trim();
    if let Some(idx) = header.find('<') {
        let name = header[..idx].trim().trim_matches('"');
        if !name.is_empty() {
            return name.to_string();
        }
    }
    // Just the email address
    header
        .trim_start_matches('<')
        .trim_end_matches('>')
        .to_string()
}

/// Best-effort email date parsing to ISO 8601
fn parse_email_date(date_str: &str) -> Option<String> {
    if date_str.is_empty() {
        return None;
    }
    // Email dates are like: "Fri, 06 Feb 2009 21:00:00 -0800"
    // Just extract enough to sort by
    let parts: Vec<&str> = date_str.split_whitespace().collect();
    if parts.len() >= 4 {
        let day = parts.iter().find(|p| p.parse::<u32>().is_ok());
        let month = parts.iter().find(|p| {
            matches!(
                p.to_lowercase().as_str(),
                "jan"
                    | "feb"
                    | "mar"
                    | "apr"
                    | "may"
                    | "jun"
                    | "jul"
                    | "aug"
                    | "sep"
                    | "oct"
                    | "nov"
                    | "dec"
            )
        });
        let year = parts
            .iter()
            .find(|p| p.len() == 4 && p.parse::<u32>().ok().map_or(false, |y| y > 1990));
        let time = parts.iter().find(|p| p.contains(':'));

        if let (Some(day), Some(month), Some(year)) = (day, month, year) {
            let m = match month.to_lowercase().as_str() {
                "jan" => "01",
                "feb" => "02",
                "mar" => "03",
                "apr" => "04",
                "may" => "05",
                "jun" => "06",
                "jul" => "07",
                "aug" => "08",
                "sep" => "09",
                "oct" => "10",
                "nov" => "11",
                "dec" => "12",
                _ => "01",
            };
            let t = time.unwrap_or(&"00:00:00");
            return Some(format!("{}-{}-{:0>2}T{}", year, m, day, t));
        }
    }

    // Fallback: return as-is
    Some(date_str.to_string())
}
