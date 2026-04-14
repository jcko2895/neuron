//! Google Takeout adapter suite — parses the full Google Takeout export.
//!
//! Handles multiple data types from a single Takeout directory:
//! - Chrome/History.json — browsing history
//! - YouTube and YouTube Music/history/watch-history.html — video watch history
//! - Calendar/*.ics — calendar events (ICS format)
//! - Contacts/All Contacts/All Contacts.vcf — contacts (VCF/vCard format)
//! - My Activity/*/MyActivity.html — search, app usage, etc.
//!
//! Takeout root typically looks like:
//! ```
//! Takeout/
//!   Calendar/
//!   Chrome/
//!   Contacts/
//!   My Activity/
//!   YouTube and YouTube Music/
//!   ...
//! ```

use crate::common::{CommonRecord, TrustLevel, format_unix_timestamp};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

pub struct GoogleTakeoutAdapter {
    user_name: String,
}

impl GoogleTakeoutAdapter {
    pub fn new(user_name: &str) -> Self {
        Self { user_name: user_name.to_string() }
    }

    // ── Chrome History ──────────────────────────────

    fn extract_chrome_history(&self, takeout_root: &Path) -> Vec<CommonRecord> {
        let mut records = Vec::new();
        let history_file = takeout_root.join("Chrome").join("History.json");
        if !history_file.exists() { return records; }

        let data = match std::fs::read_to_string(&history_file) {
            Ok(d) => d,
            Err(e) => { warn!(error = %e, "Failed to read Chrome History.json"); return records; }
        };

        let json: serde_json::Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(e) => { warn!(error = %e, "Failed to parse Chrome History.json"); return records; }
        };

        if let Some(entries) = json.get("Browser History").and_then(|v| v.as_array()) {
            for entry in entries {
                let title = entry.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let url = entry.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if title.is_empty() && url.is_empty() { continue; }

                // time_usec is Unix microseconds
                let time_usec = entry.get("time_usec").and_then(|v| v.as_u64()).unwrap_or(0);
                let timestamp = if time_usec > 0 {
                    Some(format_unix_timestamp(time_usec / 1_000_000))
                } else {
                    None
                };

                let content = if title.is_empty() {
                    format!("[Chrome] {}", url)
                } else {
                    format!("[Chrome] {} — {}", title, url)
                };

                records.push(CommonRecord {
                    content: content.clone(),
                    timestamp,
                    actor: Some(self.user_name.clone()),
                    is_user: true,
                    source_file: history_file.to_string_lossy().to_string(),
                    source_type: "google_takeout_chrome_history".into(),
                    trust_level: TrustLevel::Primary,
                    content_hash: CommonRecord::compute_content_hash(&content),
                    platform: "google_chrome".into(),
                    thread_id: None,
                    thread_name: None,
                    account: None,
                    metadata: serde_json::json!({
                        "url": url,
                        "title": title,
                    }),
                });
            }
        }

        info!(records = records.len(), "Chrome history extracted");
        records
    }

    // ── YouTube Watch History ──────────────────────────────

    fn extract_youtube_history(&self, takeout_root: &Path) -> Vec<CommonRecord> {
        let mut records = Vec::new();
        let yt_dir = takeout_root.join("YouTube and YouTube Music").join("history");
        let watch_file = yt_dir.join("watch-history.html");
        if !watch_file.exists() { return records; }

        let html = match std::fs::read_to_string(&watch_file) {
            Ok(d) => d,
            Err(e) => { warn!(error = %e, "Failed to read YouTube watch history"); return records; }
        };

        // Pattern: Watched <a href="VIDEO_URL">TITLE</a><br><a href="CHANNEL_URL">CHANNEL</a><br>DATE<br>
        // We parse these with simple string searching (no regex dependency needed)
        let mut pos = 0;
        // Google uses \u{00a0} (non-breaking space) between "Watched" and the link
        let watched_prefix = "Watched\u{00a0}<a href=\"";
        let watched_prefix_alt = "Watched <a href=\"";

        while pos < html.len() {
            let (idx, prefix_len) = if let Some(i) = html[pos..].find(watched_prefix) {
                (i, watched_prefix.len())
            } else if let Some(i) = html[pos..].find(watched_prefix_alt) {
                (i, watched_prefix_alt.len())
            } else {
                break;
            };
            let start = pos + idx + prefix_len;

            // Extract video URL
            let url_end = match html[start..].find('"') {
                Some(i) => start + i,
                None => break,
            };
            let video_url = &html[start..url_end];

            // Extract title (between > and </a>)
            let title_start = match html[url_end..].find('>') {
                Some(i) => url_end + i + 1,
                None => break,
            };
            let title_end = match html[title_start..].find("</a>") {
                Some(i) => title_start + i,
                None => break,
            };
            let title = html_decode(&html[title_start..title_end]);

            // Extract channel name (next <a href="...">CHANNEL</a>)
            let channel_search_start = title_end + 4; // skip </a>
            let channel = if let Some(ch_idx) = html[channel_search_start..].find("<a href=\"") {
                let ch_start = channel_search_start + ch_idx;
                let ch_name_start = match html[ch_start..].find('>') {
                    Some(i) => ch_start + i + 1,
                    None => { pos = channel_search_start; continue; }
                };
                let ch_name_end = match html[ch_name_start..].find("</a>") {
                    Some(i) => ch_name_start + i,
                    None => { pos = channel_search_start; continue; }
                };
                html_decode(&html[ch_name_start..ch_name_end])
            } else {
                String::new()
            };

            // Extract date (after channel </a><br>, before next <br>)
            let date_search = if !channel.is_empty() {
                match html[channel_search_start..].find("</a><br>") {
                    Some(i) => channel_search_start + i + 8,
                    None => { pos = channel_search_start; continue; }
                }
            } else {
                match html[title_end..].find("<br>") {
                    Some(i) => title_end + i + 4,
                    None => { pos = channel_search_start; continue; }
                }
            };

            let date_end = match html[date_search..].find("<br>") {
                Some(i) => date_search + i,
                None => { pos = date_search; continue; }
            };
            let date_str = html[date_search..date_end].trim().to_string();

            let content = if channel.is_empty() {
                format!("[YouTube] Watched: {}", title)
            } else {
                format!("[YouTube] Watched: {} (by {})", title, channel)
            };

            records.push(CommonRecord {
                content: content.clone(),
                timestamp: Some(date_str.clone()),
                actor: Some(self.user_name.clone()),
                is_user: true,
                source_file: watch_file.to_string_lossy().to_string(),
                source_type: "google_takeout_youtube_history".into(),
                trust_level: TrustLevel::Primary,
                content_hash: CommonRecord::compute_content_hash(&content),
                platform: "youtube".into(),
                thread_id: None,
                thread_name: None,
                account: None,
                metadata: serde_json::json!({
                    "video_url": video_url,
                    "channel": channel,
                    "raw_date": date_str,
                }),
            });

            pos = date_end;
        }

        // Also parse search history if it exists
        let search_file = yt_dir.join("search-history.html");
        if search_file.exists() {
            if let Ok(html) = std::fs::read_to_string(&search_file) {
                let search_prefix_a = "Searched for\u{00a0}";
                let search_prefix_b = "Searched for&nbsp;";
                let mut pos = 0;
                while pos < html.len() {
                    let (idx, prefix_len) = if let Some(i) = html[pos..].find(search_prefix_a) {
                        (i, search_prefix_a.len())
                    } else if let Some(i) = html[pos..].find(search_prefix_b) {
                        (i, search_prefix_b.len())
                    } else {
                        break;
                    };
                    let start = pos + idx + prefix_len;

                    // Search term might be in an <a> tag or plain text before <br>
                    let term = if html[start..].starts_with("<a ") {
                        let term_start = match html[start..].find('>') {
                            Some(i) => start + i + 1,
                            None => continue,
                        };
                        let term_end = match html[term_start..].find("</a>") {
                            Some(i) => term_start + i,
                            None => continue,
                        };
                        pos = term_end;
                        html_decode(&html[term_start..term_end])
                    } else {
                        let term_end = match html[start..].find("<br>") {
                            Some(i) => start + i,
                            None => continue,
                        };
                        pos = term_end;
                        html_decode(&html[start..term_end])
                    };

                    if term.is_empty() { continue; }

                    let content = format!("[YouTube Search] {}", term);
                    records.push(CommonRecord {
                        content: content.clone(),
                        timestamp: None, // Date parsing from nearby context is complex; skip for now
                        actor: Some(self.user_name.clone()),
                        is_user: true,
                        source_file: search_file.to_string_lossy().to_string(),
                        source_type: "google_takeout_youtube_search".into(),
                        trust_level: TrustLevel::Primary,
                        content_hash: CommonRecord::compute_content_hash(&content),
                        platform: "youtube".into(),
                        thread_id: None,
                        thread_name: None,
                        account: None,
                        metadata: serde_json::json!({}),
                    });
                }
            }
        }

        info!(records = records.len(), "YouTube history extracted");
        records
    }

    // ── Calendar (ICS) ──────────────────────────────

    fn extract_calendar(&self, takeout_root: &Path) -> Vec<CommonRecord> {
        let mut records = Vec::new();
        let cal_dir = takeout_root.join("Calendar");
        if !cal_dir.exists() { return records; }

        let ics_files: Vec<PathBuf> = match std::fs::read_dir(&cal_dir) {
            Ok(entries) => entries.flatten()
                .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("ics"))
                .map(|e| e.path())
                .collect(),
            Err(_) => return records,
        };

        for ics_path in &ics_files {
            let data = match std::fs::read_to_string(ics_path) {
                Ok(d) => d,
                Err(e) => { warn!(error = %e, file = %ics_path.display(), "Failed to read ICS"); continue; }
            };

            let cal_name = ics_path.file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            // Parse VEVENT blocks
            let mut in_event = false;
            let mut summary = String::new();
            let mut dtstart = String::new();
            let mut dtend = String::new();
            let mut location = String::new();
            let mut description = String::new();

            for line in data.lines() {
                let line = line.trim_end();
                if line == "BEGIN:VEVENT" {
                    in_event = true;
                    summary.clear();
                    dtstart.clear();
                    dtend.clear();
                    location.clear();
                    description.clear();
                } else if line == "END:VEVENT" && in_event {
                    in_event = false;
                    if summary.is_empty() { continue; }

                    let timestamp = if !dtstart.is_empty() {
                        Some(ics_datetime_to_iso(&dtstart))
                    } else {
                        None
                    };

                    let mut content = format!("[Calendar: {}] {}", cal_name, ics_unescape(&summary));
                    if !location.is_empty() {
                        content.push_str(&format!(" @ {}", ics_unescape(&location)));
                    }

                    records.push(CommonRecord {
                        content: content.clone(),
                        timestamp,
                        actor: Some(self.user_name.clone()),
                        is_user: true,
                        source_file: ics_path.to_string_lossy().to_string(),
                        source_type: "google_takeout_calendar".into(),
                        trust_level: TrustLevel::Primary,
                        content_hash: CommonRecord::compute_content_hash(&content),
                        platform: "google_calendar".into(),
                        thread_id: Some(cal_name.clone()),
                        thread_name: Some(cal_name.clone()),
                        account: None,
                        metadata: serde_json::json!({
                            "dtstart": dtstart,
                            "dtend": dtend,
                            "location": ics_unescape(&location),
                            "description": ics_unescape(&description),
                        }),
                    });
                } else if in_event {
                    if let Some(val) = line.strip_prefix("SUMMARY:") {
                        summary = val.to_string();
                    } else if let Some(val) = line.strip_prefix("DTSTART:") {
                        dtstart = val.to_string();
                    } else if let Some(val) = line.strip_prefix("DTSTART;") {
                        // DTSTART;VALUE=DATE:20230115 or DTSTART;TZID=America/Los_Angeles:20230115T090000
                        if let Some(colon_pos) = val.find(':') {
                            dtstart = val[colon_pos + 1..].to_string();
                        }
                    } else if let Some(val) = line.strip_prefix("DTEND:") {
                        dtend = val.to_string();
                    } else if let Some(val) = line.strip_prefix("DTEND;") {
                        if let Some(colon_pos) = val.find(':') {
                            dtend = val[colon_pos + 1..].to_string();
                        }
                    } else if let Some(val) = line.strip_prefix("LOCATION:") {
                        location = val.to_string();
                    } else if let Some(val) = line.strip_prefix("DESCRIPTION:") {
                        description = val.to_string();
                    }
                }
            }
        }

        info!(records = records.len(), "Calendar events extracted");
        records
    }

    // ── Contacts (VCF) ──────────────────────────────

    fn extract_contacts(&self, takeout_root: &Path) -> Vec<CommonRecord> {
        let mut records = Vec::new();
        let contacts_dir = takeout_root.join("Contacts");
        if !contacts_dir.exists() { return records; }

        // Find VCF files (usually in All Contacts/)
        let mut vcf_files = Vec::new();
        fn find_vcf(dir: &Path, out: &mut Vec<PathBuf>) {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        find_vcf(&path, out);
                    } else if path.extension().and_then(|x| x.to_str()) == Some("vcf") {
                        out.push(path);
                    }
                }
            }
        }
        find_vcf(&contacts_dir, &mut vcf_files);

        for vcf_path in &vcf_files {
            let data = match std::fs::read_to_string(vcf_path) {
                Ok(d) => d,
                Err(e) => { warn!(error = %e, file = %vcf_path.display(), "Failed to read VCF"); continue; }
            };

            let mut in_card = false;
            let mut fn_name = String::new();
            let mut org = String::new();
            let mut emails: Vec<String> = Vec::new();
            let mut phones: Vec<String> = Vec::new();

            for line in data.lines() {
                let line = line.trim_end();
                if line == "BEGIN:VCARD" {
                    in_card = true;
                    fn_name.clear();
                    org.clear();
                    emails.clear();
                    phones.clear();
                } else if line == "END:VCARD" && in_card {
                    in_card = false;
                    let name = if !fn_name.is_empty() {
                        fn_name.clone()
                    } else if !org.is_empty() {
                        org.clone()
                    } else {
                        continue; // Skip contacts with no name or org
                    };

                    let mut parts = vec![format!("[Contact] {}", name)];
                    if !org.is_empty() && org != name { parts.push(format!("Org: {}", org)); }
                    for e in &emails { parts.push(format!("Email: {}", e)); }
                    for p in &phones { parts.push(format!("Phone: {}", p)); }
                    let content = parts.join(" | ");

                    records.push(CommonRecord {
                        content: content.clone(),
                        timestamp: None,
                        actor: Some(name.clone()),
                        is_user: false,
                        source_file: vcf_path.to_string_lossy().to_string(),
                        source_type: "google_takeout_contacts".into(),
                        trust_level: TrustLevel::Primary,
                        content_hash: CommonRecord::compute_content_hash(&content),
                        platform: "google_contacts".into(),
                        thread_id: None,
                        thread_name: None,
                        account: None,
                        metadata: serde_json::json!({
                            "name": name,
                            "org": org,
                            "emails": emails,
                            "phones": phones,
                        }),
                    });
                } else if in_card {
                    if let Some(val) = line.strip_prefix("FN:") {
                        fn_name = val.to_string();
                    } else if let Some(val) = line.strip_prefix("ORG:") {
                        org = val.trim_end_matches(';').to_string();
                    } else if line.contains("EMAIL") {
                        // EMAIL;TYPE=INTERNET:foo@bar.com or item1.EMAIL;TYPE=INTERNET:...
                        if let Some(colon_pos) = line.find(':') {
                            let email = line[colon_pos + 1..].trim().to_string();
                            if !email.is_empty() { emails.push(email); }
                        }
                    } else if line.starts_with("TEL") {
                        // TEL:(206) 209-8756 or TEL;TYPE=CELL:(425) 800-7796
                        if let Some(colon_pos) = line.find(':') {
                            let phone = line[colon_pos + 1..].trim().to_string();
                            if !phone.is_empty() { phones.push(phone); }
                        }
                    }
                }
            }
        }

        info!(records = records.len(), "Google Contacts extracted");
        records
    }

    // ── My Activity (HTML) ──────────────────────────────

    fn extract_my_activity(&self, takeout_root: &Path) -> Vec<CommonRecord> {
        let mut records = Vec::new();
        let activity_dir = takeout_root.join("My Activity");
        if !activity_dir.exists() { return records; }

        // Scan all subdirectories for MyActivity.html files
        let mut html_files = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&activity_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let activity_file = path.join("MyActivity.html");
                    if activity_file.exists() {
                        html_files.push((
                            entry.file_name().to_string_lossy().to_string(),
                            activity_file,
                        ));
                    }
                }
            }
        }

        for (category, file_path) in &html_files {
            let html = match std::fs::read_to_string(file_path) {
                Ok(d) => d,
                Err(e) => { warn!(error = %e, category = %category, "Failed to read MyActivity.html"); continue; }
            };

            // Google Activity HTML uses content-cell divs
            // Pattern: "Searched for <a ...>QUERY</a>" or "Visited <a ...>URL</a>" or plain text activities
            // We extract text between content-cell markers

            let search_prefixes = ["Searched for\u{00a0}", "Searched for&nbsp;"];
            let visited_prefixes = ["Visited\u{00a0}", "Visited&nbsp;"];

            let mut pos = 0;
            while pos < html.len() {
                // Try to find "Searched for" entries
                let search_match = search_prefixes.iter()
                    .filter_map(|p| html[pos..].find(p).map(|i| (i, p.len())))
                    .min_by_key(|(i, _)| *i);
                let visited_match = visited_prefixes.iter()
                    .filter_map(|p| html[pos..].find(p).map(|i| (i, p.len())))
                    .min_by_key(|(i, _)| *i);

                // Pick whichever comes first
                let (is_search, idx, prefix_len) = match (search_match, visited_match) {
                    (Some((si, sl)), Some((vi, vl))) => {
                        if si <= vi { (true, si, sl) } else { (false, vi, vl) }
                    }
                    (Some((si, sl)), None) => (true, si, sl),
                    (None, Some((vi, vl))) => (false, vi, vl),
                    (None, None) => break,
                };

                if is_search {
                    let start = pos + idx + prefix_len;

                    let term = if html[start..].starts_with("<a ") {
                        let term_start = match html[start..].find('>') {
                            Some(i) => start + i + 1,
                            None => continue,
                        };
                        let term_end = match html[term_start..].find("</a>") {
                            Some(i) => term_start + i,
                            None => continue,
                        };
                        pos = term_end;
                        html_decode(&html[term_start..term_end])
                    } else {
                        let term_end = match html[start..].find("<") {
                            Some(i) => start + i,
                            None => continue,
                        };
                        pos = term_end;
                        html_decode(&html[start..term_end])
                    };

                    if term.is_empty() { continue; }

                    // Try to find date after the search term
                    let date = extract_nearby_date(&html, pos);

                    let content = format!("[Google {} Search] {}", category, term);
                    records.push(CommonRecord {
                        content: content.clone(),
                        timestamp: date,
                        actor: Some(self.user_name.clone()),
                        is_user: true,
                        source_file: file_path.to_string_lossy().to_string(),
                        source_type: format!("google_takeout_activity_{}", category.to_lowercase()),
                        trust_level: TrustLevel::Primary,
                        content_hash: CommonRecord::compute_content_hash(&content),
                        platform: "google_activity".into(),
                        thread_id: Some(category.clone()),
                        thread_name: Some(category.clone()),
                        account: None,
                        metadata: serde_json::json!({}),
                    });
                } else {
                    // Visited entry
                    let start = pos + idx + prefix_len;

                    let title = if html[start..].starts_with("<a ") {
                        let t_start = match html[start..].find('>') {
                            Some(i) => start + i + 1,
                            None => continue,
                        };
                        let t_end = match html[t_start..].find("</a>") {
                            Some(i) => t_start + i,
                            None => continue,
                        };
                        pos = t_end;
                        html_decode(&html[t_start..t_end])
                    } else {
                        let t_end = match html[start..].find("<") {
                            Some(i) => start + i,
                            None => continue,
                        };
                        pos = t_end;
                        html_decode(&html[start..t_end])
                    };

                    if title.is_empty() { continue; }

                    let date = extract_nearby_date(&html, pos);

                    let content = format!("[Google {} Visit] {}", category, title);
                    records.push(CommonRecord {
                        content: content.clone(),
                        timestamp: date,
                        actor: Some(self.user_name.clone()),
                        is_user: true,
                        source_file: file_path.to_string_lossy().to_string(),
                        source_type: format!("google_takeout_activity_{}", category.to_lowercase()),
                        trust_level: TrustLevel::Primary,
                        content_hash: CommonRecord::compute_content_hash(&content),
                        platform: "google_activity".into(),
                        thread_id: Some(category.clone()),
                        thread_name: Some(category.clone()),
                        account: None,
                        metadata: serde_json::json!({}),
                    });
                }
            }
        }

        info!(records = records.len(), "Google My Activity extracted");
        records
    }
}

impl super::SourceAdapter for GoogleTakeoutAdapter {
    fn name(&self) -> &str { "Google Takeout" }
    fn platform(&self) -> &str { "google" }

    fn can_handle_file(&self, path: &Path) -> bool {
        // Takeout root has characteristic directories
        path.join("Chrome").exists()
            || path.join("Calendar").exists()
            || path.join("Contacts").exists()
            || path.join("My Activity").exists()
            || path.join("YouTube and YouTube Music").exists()
    }

    fn extract_from_file(&self, path: &Path) -> Result<Vec<CommonRecord>, String> {
        let mut all_records = Vec::new();

        all_records.extend(self.extract_chrome_history(path));
        all_records.extend(self.extract_youtube_history(path));
        all_records.extend(self.extract_calendar(path));
        all_records.extend(self.extract_contacts(path));
        all_records.extend(self.extract_my_activity(path));

        info!(total = all_records.len(), "Google Takeout extraction complete");
        Ok(all_records)
    }

    fn discover_local(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let candidates = [
            PathBuf::from("G:/staging/google-takeout/Takeout"),
        ];
        for p in &candidates {
            if self.can_handle_file(p) { found.push(p.clone()); }
        }
        found
    }
}

// ── Helpers ──────────────────────────────

/// Decode basic HTML entities.
fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
     .replace("&lt;", "<")
     .replace("&gt;", ">")
     .replace("&quot;", "\"")
     .replace("&#39;", "'")
     .replace("&nbsp;", " ")
     .replace("&emsp;", " ")
     .replace("\u{202f}", " ") // narrow no-break space
}

/// Convert ICS datetime to ISO 8601.
/// Input: "20230115T090000" or "20230115" or "20230115T090000Z"
fn ics_datetime_to_iso(dt: &str) -> String {
    let dt = dt.trim();
    if dt.len() >= 15 && dt.contains('T') {
        // 20230115T090000 or 20230115T090000Z
        let base = &dt[..15];
        let tz = if dt.ends_with('Z') { "+00:00" } else { "" };
        format!(
            "{}-{}-{}T{}:{}:{}{}",
            &base[0..4], &base[4..6], &base[6..8],
            &base[9..11], &base[11..13], &base[13..15],
            tz
        )
    } else if dt.len() >= 8 {
        // Date only: 20230115
        format!("{}-{}-{}", &dt[0..4], &dt[4..6], &dt[6..8])
    } else {
        dt.to_string()
    }
}

/// Unescape ICS backslash escapes.
fn ics_unescape(s: &str) -> String {
    s.replace("\\n", "\n")
     .replace("\\,", ",")
     .replace("\\;", ";")
     .replace("\\\\", "\\")
}

/// Try to extract a date string from nearby HTML content.
/// Google Takeout dates look like "Jan 5, 2026, 10:31:59\u{202f}PM PST"
fn extract_nearby_date(html: &str, pos: usize) -> Option<String> {
    // Look for a date pattern within the next 500 chars
    let search_end = (pos + 500).min(html.len());
    let slice = &html[pos..search_end];

    // Google dates appear between <br> tags, format: "Mon DD, YYYY, HH:MM:SS AM/PM TZ"
    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

    for month in &months {
        if let Some(idx) = slice.find(month) {
            // Find the end of the date (next <br> or <)
            let date_start = idx;
            let date_end = slice[date_start..].find("<br>")
                .or_else(|| slice[date_start..].find("<"))
                .map(|i| date_start + i)
                .unwrap_or(slice.len().min(date_start + 50));
            let date_str = html_decode(slice[date_start..date_end].trim());
            if date_str.len() > 8 {
                return Some(date_str);
            }
        }
    }
    None
}
