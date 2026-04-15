//! Platform friend/username mapping readers.
//!
//! Each platform export includes a way to map usernames to display names.
//! This module reads those mappings to feed into the identity resolver.

use std::collections::HashMap;
use std::path::Path;
use tracing::{info, warn};

/// Username → display name.
pub type UsernameMap = HashMap<String, String>;

/// Read Snapchat friends.json → username → display name.
pub fn read_snapchat_friends(path: &Path) -> UsernameMap {
    let mut map = UsernameMap::new();

    let data = match std::fs::read_to_string(path) {
        Ok(d) => d,
        Err(e) => {
            // Try reading from a zip file
            if path.extension().and_then(|e| e.to_str()) == Some("zip") {
                match read_snapchat_friends_from_zip(path) {
                    Ok(m) => return m,
                    Err(e) => { warn!(error = %e, "Failed to read Snapchat friends from zip"); return map; }
                }
            }
            warn!(error = %e, "Failed to read Snapchat friends.json");
            return map;
        }
    };

    parse_snapchat_friends_json(&data, &mut map);
    info!(entries = map.len(), "Snapchat friends loaded");
    map
}

fn read_snapchat_friends_from_zip(path: &Path) -> Result<UsernameMap, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut friends_file = archive.by_name("json/friends.json").map_err(|e| e.to_string())?;
    let mut data = String::new();
    std::io::Read::read_to_string(&mut friends_file, &mut data).map_err(|e| e.to_string())?;
    let mut map = UsernameMap::new();
    parse_snapchat_friends_json(&data, &mut map);
    info!(entries = map.len(), "Snapchat friends loaded from zip");
    Ok(map)
}

fn parse_snapchat_friends_json(data: &str, map: &mut UsernameMap) {
    let json: serde_json::Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return,
    };

    if let Some(friends) = json.get("Friends").and_then(|v| v.as_array()) {
        for f in friends {
            let username = f.get("Username").and_then(|v| v.as_str()).unwrap_or("");
            let display = f.get("Display Name").and_then(|v| v.as_str()).unwrap_or("");
            if !username.is_empty() && !display.is_empty() {
                map.insert(username.to_string(), display.to_string());
            }
        }
    }
}

/// Read Instagram thread → display name mappings.
/// Instagram exports use thread IDs like "username_12345" — we can extract the username.
/// But the display name comes from the message actor field.
/// This function scans Neuron JSONL output for Instagram-specific actor→thread mappings.
pub fn read_instagram_actors(jsonl_path: &Path) -> UsernameMap {
    let mut map = UsernameMap::new();

    let file = match std::fs::File::open(jsonl_path) {
        Ok(f) => f,
        Err(_) => return map,
    };

    use std::io::BufRead;
    let reader = std::io::BufReader::new(file);
    for line in reader.lines().flatten() {
        let record: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if record.get("platform").and_then(|v| v.as_str()) != Some("instagram") {
            continue;
        }

        let actor = record.get("actor").and_then(|v| v.as_str()).unwrap_or("");
        let thread = record.get("thread_id").or_else(|| record.get("thread_name"))
            .and_then(|v| v.as_str()).unwrap_or("");

        if !actor.is_empty() && !thread.is_empty() && !record.get("is_user").and_then(|v| v.as_bool()).unwrap_or(false) {
            // Thread name often contains the username: "username_12345"
            if let Some(username) = thread.split('_').next() {
                if !username.is_empty() && username.len() > 2 {
                    map.entry(username.to_string()).or_insert_with(|| actor.to_string());
                }
            }
        }
    }

    info!(entries = map.len(), "Instagram actors loaded");
    map
}
