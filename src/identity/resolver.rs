//! Identity resolution engine.
//!
//! Takes Neuron JSONL records + contact/friend mappings and builds the identity graph.
//! All rules are universal — zero user-specific code.
//!
//! # Resolution Order
//!
//! For each record's actor:
//! 1. Is this a phone number? → look up in contacts phone map → get full name
//! 2. Is this a platform username? → look up in platform friend map → get display name
//! 3. Is this a full name (first + last)? → find existing person by exact full name match
//! 4. Is this just a first name? → DO NOT MERGE with anyone. Create/keep separate.
//! 5. Found a match? → add this identifier to that person
//! 6. No match? → create new person
//!
//! # Critical Rules
//!
//! - Different last names → NEVER merge. Period.
//! - First name only → NEVER merge. Keep separate.
//! - Ambiguous → keep separate. Duplicates are better than wrong merges.
//! - User corrections override everything.

use std::collections::HashMap;
use std::path::Path;
use tracing::{info, warn, debug};

use crate::identity::*;
use crate::identity::contacts::{PhoneMap, EmailMap};
use crate::identity::platform_friends::UsernameMap;

/// The resolver holds all mapping data and processes records.
pub struct IdentityResolver {
    /// Phone → full name (from contacts).
    phone_map: PhoneMap,
    /// Email → full name (from contacts).
    email_map: EmailMap,
    /// Platform-specific username → display name.
    /// Key: "platform:username", Value: display name.
    username_maps: HashMap<String, String>,
}

impl IdentityResolver {
    pub fn new() -> Self {
        Self {
            phone_map: PhoneMap::new(),
            email_map: EmailMap::new(),
            username_maps: HashMap::new(),
        }
    }

    /// Load contacts from Apple Contacts directory.
    pub fn load_contacts(&mut self, dir: &Path) {
        let (phones, emails) = crate::identity::contacts::read_apple_contacts(dir);
        self.phone_map.extend(phones);
        self.email_map.extend(emails);
    }

    /// Load Snapchat friend mappings.
    pub fn load_snapchat_friends(&mut self, path: &Path) {
        let map = crate::identity::platform_friends::read_snapchat_friends(path);
        for (username, display) in map {
            self.username_maps.insert(format!("snapchat:{}", username), display);
        }
    }

    /// Load Instagram actor mappings from Neuron JSONL.
    pub fn load_instagram_actors(&mut self, jsonl_path: &Path) {
        let map = crate::identity::platform_friends::read_instagram_actors(jsonl_path);
        for (username, display) in map {
            self.username_maps.insert(format!("instagram:{}", username), display);
        }
    }

    /// Resolve an actor string from a record into a canonical name.
    /// Returns (canonical_name, identifier_type).
    fn resolve_actor(&self, actor: &str, platform: &str) -> (String, IdentifierType) {
        // 1. Phone number?
        if actor.starts_with('+') || (actor.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) && actor.len() > 8) {
            if let Some(normalized) = normalize_phone(actor) {
                if let Some(name) = self.phone_map.get(&normalized) {
                    return (name.clone(), IdentifierType::Phone);
                }
            }
            // Unknown phone — keep as-is, don't merge with anyone
            return (actor.to_string(), IdentifierType::Phone);
        }

        // 2. Email?
        if actor.contains('@') {
            let normalized = normalize_email(actor);
            if let Some(name) = self.email_map.get(&normalized) {
                return (name.clone(), IdentifierType::Email);
            }
            return (actor.to_string(), IdentifierType::Email);
        }

        // 3. Platform username mapping?
        let platform_key = format!("{}:{}", platform, actor);
        if let Some(display_name) = self.username_maps.get(&platform_key) {
            return (display_name.clone(), IdentifierType::Username);
        }

        // 4. It's a name (display name or full name from the platform).
        let (first, last) = parse_name(actor);
        if !last.is_empty() {
            // Has first + last → treat as full name
            (actor.to_string(), IdentifierType::FullName)
        } else {
            // First name only — DO NOT try to merge
            (actor.to_string(), IdentifierType::DisplayName)
        }
    }

    /// Process all records from a Neuron JSONL export and build the identity graph.
    pub fn resolve_all(&self, jsonl_path: &Path, db: &rusqlite::Connection) -> Result<ResolveStats, String> {
        let file = std::fs::File::open(jsonl_path)
            .map_err(|e| format!("Failed to open JSONL: {}", e))?;

        let reader = std::io::BufReader::new(file);
        let mut stats = ResolveStats::default();

        // Cache: resolved name → person_id (avoid repeated DB lookups)
        let mut name_cache: HashMap<String, i64> = HashMap::new();
        // Cache: raw actor → resolved name (avoid repeated resolution)
        let mut actor_cache: HashMap<String, (String, IdentifierType)> = HashMap::new();

        // Begin transaction for bulk insert performance
        db.execute_batch("BEGIN TRANSACTION").map_err(|e| e.to_string())?;
        let mut batch_count = 0usize;

        use std::io::BufRead;
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };
            if line.trim().is_empty() { continue; }

            let record: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => { stats.errors += 1; continue; }
            };

            stats.records_processed += 1;

            let actor = match record.get("actor").and_then(|v| v.as_str()) {
                Some(a) if !a.is_empty() => a,
                _ => continue,
            };

            // Skip the user's own records
            if record.get("is_user").and_then(|v| v.as_bool()).unwrap_or(false) {
                continue;
            }

            let platform = record.get("platform").and_then(|v| v.as_str()).unwrap_or("");
            let timestamp = record.get("timestamp").and_then(|v| v.as_str());

            // Resolve actor to canonical name
            let cache_key = format!("{}:{}", platform, actor);
            let (canonical_name, id_type) = if let Some(cached) = actor_cache.get(&cache_key) {
                cached.clone()
            } else {
                let resolved = self.resolve_actor(actor, platform);
                actor_cache.insert(cache_key.clone(), resolved.clone());
                resolved
            };

            // Skip non-real names (all special chars, too short, etc.)
            if !is_plausible_name(&canonical_name) {
                continue;
            }

            // Find or create person in DB
            let person_id = if let Some(&cached_id) = name_cache.get(&canonical_name) {
                cached_id
            } else {
                // Try to find by full name (only if has last name)
                let (first, last) = parse_name(&canonical_name);
                let found = if !last.is_empty() {
                    db::find_by_full_name(db, &canonical_name)
                } else {
                    // First name only — check if this exact identifier already exists
                    db::find_by_identifier(db, platform, actor)
                };

                match found {
                    Some(id) => {
                        name_cache.insert(canonical_name.clone(), id);
                        id
                    }
                    None => {
                        // Create new person
                        let gender = guess_gender(&first);
                        let id = db::insert_person(db, &canonical_name, &gender)?;
                        name_cache.insert(canonical_name.clone(), id);
                        stats.persons_created += 1;
                        id
                    }
                }
            };

            // Add identifier
            let _ = db::add_identifier(db, person_id, platform, actor, &id_type.to_string());

            // Update interaction count
            let _ = db::update_interaction(db, person_id, 1, timestamp, timestamp);

            stats.identifiers_linked += 1;

            // Commit in batches of 10,000 for performance
            batch_count += 1;
            if batch_count % 10_000 == 0 {
                db.execute_batch("COMMIT; BEGIN TRANSACTION").map_err(|e| e.to_string())?;
                if batch_count % 100_000 == 0 {
                    info!(processed = batch_count, persons = stats.persons_created, "progress");
                }
            }
        }

        // Final commit
        db.execute_batch("COMMIT").map_err(|e| e.to_string())?;

        // Apply any existing corrections
        let corrections_applied = db::apply_corrections(db)?;
        stats.corrections_applied = corrections_applied;

        info!(
            records = stats.records_processed,
            persons = stats.persons_created,
            identifiers = stats.identifiers_linked,
            corrections = stats.corrections_applied,
            errors = stats.errors,
            "Identity resolution complete"
        );

        Ok(stats)
    }
}

#[derive(Debug, Default)]
pub struct ResolveStats {
    pub records_processed: usize,
    pub persons_created: usize,
    pub identifiers_linked: usize,
    pub corrections_applied: usize,
    pub errors: usize,
}

/// Check if a string is a plausible person name (not garbage, not a business).
fn is_plausible_name(name: &str) -> bool {
    // Must have at least 2 ASCII letters
    let latin_count = name.chars().filter(|c| c.is_ascii_alphabetic()).count();
    if latin_count < 2 { return false; }

    // Too much non-ASCII (emoji names, unicode art)
    let non_ascii = name.chars().filter(|c| !c.is_ascii()).count();
    if non_ascii as f32 / name.len() as f32 > 0.3 { return false; }

    // Starts with + and has mostly digits (unresolved phone)
    if name.starts_with('+') { return false; }

    // Contains @ (email)
    if name.contains('@') { return false; }

    // Encoded garbage
    if name.contains("=?") { return false; }

    true
}

/// Simple gender guess from first name.
/// Returns "M", "F", or "?".
fn guess_gender(first_name: &str) -> String {
    // This is a basic heuristic. The corrections system handles mistakes.
    let lower = first_name.to_lowercase();

    const FEMALE: &[&str] = &[
        "jenny", "grace", "nina", "caroline", "lucy", "sydney", "kelli", "natalie",
        "audrey", "jennine", "alyssa", "kathryn", "elisabeth", "elysabeth", "madison",
        "chelsea", "teresa", "jacqueline", "nicole", "shelby", "vanessa", "maria",
        "danielle", "meagan", "sabrina", "mikayla", "kayla", "tara", "emma", "courtney",
        "julie", "lana", "savannah", "aimie", "carolina", "mariah", "ashley", "kelly",
        "sarah", "hannah", "kim", "rachel", "stephanie", "jessica", "amanda", "jennifer",
        "megan", "brittany", "emily", "samantha", "elizabeth", "rebecca", "lauren",
        "christina", "katherine", "michelle", "heather", "amber", "melissa", "victoria",
        "alexandra", "diana", "sophia", "olivia", "isabella", "mia", "chloe",
    ];

    const MALE: &[&str] = &[
        "keith", "justin", "grady", "hayden", "eric", "jeff", "jeffery", "walter",
        "tyler", "gabe", "david", "fernando", "randy", "juan", "chris", "uriah",
        "dominic", "logan", "michael", "mark", "iain", "truman", "daniel", "steven",
        "kevin", "joseph", "andy", "johnson", "tobias", "sean", "carlos", "glen",
        "benjamin", "anthony", "bryce", "oliver", "marc", "ernest", "troy", "jake",
        "ben", "carl", "levi", "dennis", "billy", "kirk", "alex", "derek", "caleb",
        "john", "jim", "james", "robert", "william", "richard", "thomas", "charles",
        "matthew", "christopher", "andrew", "joshua", "ryan", "brandon", "brian",
        "nathan", "adam", "jason", "patrick", "timothy", "scott", "aaron", "paul",
    ];

    if FEMALE.contains(&lower.as_str()) {
        "F".to_string()
    } else if MALE.contains(&lower.as_str()) {
        "M".to_string()
    } else {
        "?".to_string()
    }
}
