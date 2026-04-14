//! Entity Resolution — merge people across platforms into unified identities.
//!
//! The same person appears differently on every platform:
//! - Facebook: "Eric Hemmen"
//! - Gmail: "starfleet.command@live.com"
//! - iMessage: "+14257360188"
//! - Claude session: "Eric"
//!
//! Entity resolution merges these into one Person entity with all identifiers,
//! and traces every interaction back to the source record.
//!
//! The people graph shows: who you know, when you met, how close you are,
//! when the relationship faded, and the evidence for all of it.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::info;

use crate::common::CommonRecord;

/// A unique identifier for a person on a specific platform.
#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonIdentifier {
    /// What type of identifier (name, email, phone, username)
    pub id_type: IdentifierType,
    /// The actual value
    pub value: String,
    /// Which platform this was seen on
    pub platform: String,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdentifierType {
    Name,
    Email,
    Phone,
    Username,
    FacebookId,
}

/// A unified person — merged across all platforms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Person {
    /// Internal ID
    pub id: String,
    /// Primary display name (most frequently used)
    pub display_name: String,
    /// Is this a real person, business, or automated sender?
    pub entity_type: EntityType,
    /// All known identifiers across all platforms
    pub identifiers: Vec<PersonIdentifier>,
    /// Relationship to the user
    pub relationship: Option<String>,
    /// When first seen (earliest record)
    pub first_seen: Option<String>,
    /// When last seen (most recent record)
    pub last_seen: Option<String>,
    /// Total interactions (messages sent + received)
    pub interaction_count: u64,
    /// Platforms this person appears on
    pub platforms: Vec<String>,
    /// User-provided notes (e.g., "older girl from Church, artist")
    pub notes: Option<String>,
    /// Interaction timeline: year → count
    pub timeline: HashMap<String, u64>,
    /// Source record IDs that reference this person (provenance)
    pub source_records: Vec<String>,
}

/// The people graph — all resolved persons and their interactions.
pub struct PeopleGraph {
    /// All known persons, keyed by internal ID
    persons: HashMap<String, Person>,
    /// Identifier → person ID mapping (for dedup/merge)
    id_index: HashMap<PersonIdentifier, String>,
    /// Next person ID counter
    next_id: u64,
}

impl PeopleGraph {
    pub fn new() -> Self {
        Self {
            persons: HashMap::new(),
            id_index: HashMap::new(),
            next_id: 1,
        }
    }

    /// Find or create a person from an identifier.
    /// If the identifier is already known, returns the existing person.
    /// If not, creates a new person.
    pub fn resolve(&mut self, identifier: PersonIdentifier) -> String {
        // Check if this identifier is already mapped to a person
        if let Some(person_id) = self.id_index.get(&identifier) {
            return person_id.clone();
        }

        // Try fuzzy name matching — if we have a name identifier,
        // check if any existing person has a similar name
        if identifier.id_type == IdentifierType::Name {
            let name_lower = identifier.value.to_lowercase();
            for (pid, person) in &self.persons {
                for ident in &person.identifiers {
                    if ident.id_type == IdentifierType::Name
                        && ident.value.to_lowercase() == name_lower
                    {
                        // Same name on different platform — merge
                        self.id_index.insert(identifier, pid.clone());
                        return pid.clone();
                    }
                }
            }
        }

        // New person
        let person_id = format!("person_{}", self.next_id);
        self.next_id += 1;

        let display_name = match identifier.id_type {
            IdentifierType::Name => identifier.value.clone(),
            _ => identifier.value.clone(),
        };

        let platform = identifier.platform.clone();

        // Classify entity type
        let entity_type = classify_entity(&display_name, &identifier.value);

        let person = Person {
            id: person_id.clone(),
            display_name,
            entity_type,
            identifiers: vec![identifier.clone()],
            relationship: None,
            first_seen: None,
            last_seen: None,
            interaction_count: 0,
            platforms: vec![platform],
            notes: None,
            timeline: HashMap::new(),
            source_records: Vec::new(),
        };

        self.persons.insert(person_id.clone(), person);
        self.id_index.insert(identifier, person_id.clone());
        person_id
    }

    /// Merge two persons into one (when we discover they're the same person).
    /// Keeps the first person's ID, merges all data from the second.
    pub fn merge(&mut self, keep_id: &str, merge_id: &str) {
        if keep_id == merge_id {
            return;
        }

        let merge_person = match self.persons.remove(merge_id) {
            Some(p) => p,
            None => return,
        };

        if let Some(keep_person) = self.persons.get_mut(keep_id) {
            // Merge identifiers
            for ident in &merge_person.identifiers {
                if !keep_person.identifiers.contains(ident) {
                    keep_person.identifiers.push(ident.clone());
                }
                // Update index to point to kept person
                self.id_index.insert(ident.clone(), keep_id.to_string());
            }

            // Merge platforms
            for platform in &merge_person.platforms {
                if !keep_person.platforms.contains(platform) {
                    keep_person.platforms.push(platform.clone());
                }
            }

            // Merge interaction count
            keep_person.interaction_count += merge_person.interaction_count;

            // Merge timeline
            for (year, count) in &merge_person.timeline {
                *keep_person.timeline.entry(year.clone()).or_insert(0) += count;
            }

            // Merge source records
            keep_person
                .source_records
                .extend(merge_person.source_records);

            // Keep earlier first_seen
            if let Some(ref merge_first) = merge_person.first_seen {
                if keep_person.first_seen.is_none()
                    || keep_person.first_seen.as_deref() > Some(merge_first.as_str())
                {
                    keep_person.first_seen = Some(merge_first.clone());
                }
            }

            // Keep later last_seen
            if let Some(ref merge_last) = merge_person.last_seen {
                if keep_person.last_seen.is_none()
                    || keep_person.last_seen.as_deref() < Some(merge_last.as_str())
                {
                    keep_person.last_seen = Some(merge_last.clone());
                }
            }
        }
    }

    /// Link a known identifier to an existing person.
    /// Use when you discover that an email belongs to a known person.
    pub fn link_identifier(&mut self, person_id: &str, identifier: PersonIdentifier) {
        self.id_index
            .insert(identifier.clone(), person_id.to_string());
        if let Some(person) = self.persons.get_mut(person_id) {
            if !person.identifiers.contains(&identifier) {
                person.identifiers.push(identifier.clone());
            }
            if !person.platforms.contains(&identifier.platform) {
                person.platforms.push(identifier.platform.clone());
            }
        }
    }

    /// Record an interaction with a person from a CommonRecord.
    pub fn record_interaction(&mut self, person_id: &str, record: &CommonRecord) {
        if let Some(person) = self.persons.get_mut(person_id) {
            person.interaction_count += 1;

            // Update first/last seen
            if let Some(ref ts) = record.timestamp {
                if person.first_seen.is_none() || person.first_seen.as_deref() > Some(ts.as_str()) {
                    person.first_seen = Some(ts.clone());
                }
                if person.last_seen.is_none() || person.last_seen.as_deref() < Some(ts.as_str()) {
                    person.last_seen = Some(ts.clone());
                }

                // Timeline: extract year
                if ts.len() >= 4 {
                    let year = &ts[..4];
                    *person.timeline.entry(year.to_string()).or_insert(0) += 1;
                }
            }

            // Track source record
            person.source_records.push(record.id());
        }
    }

    /// Process a batch of CommonRecords and build the people graph.
    pub fn process_records(&mut self, records: &[CommonRecord]) {
        for record in records {
            // Extract person from the actor field
            if let Some(ref actor) = record.actor {
                if record.is_user {
                    continue; // Skip the user themselves
                }

                let identifier = PersonIdentifier {
                    id_type: classify_identifier(actor),
                    value: actor.clone(),
                    platform: record.platform.clone(),
                };

                let person_id = self.resolve(identifier);
                self.record_interaction(&person_id, record);
            }
        }
    }

    /// Get all persons sorted by interaction count (most interactions first).
    pub fn all_persons_sorted(&self) -> Vec<&Person> {
        let mut persons: Vec<&Person> = self.persons.values().collect();
        persons.sort_by(|a, b| b.interaction_count.cmp(&a.interaction_count));
        persons
    }

    /// Get a person by ID.
    pub fn get(&self, id: &str) -> Option<&Person> {
        self.persons.get(id)
    }

    /// Find a person by any identifier value.
    pub fn find_by_identifier(&self, value: &str) -> Option<&Person> {
        for (ident, person_id) in &self.id_index {
            if ident.value == value || ident.value.to_lowercase() == value.to_lowercase() {
                return self.persons.get(person_id);
            }
        }
        None
    }

    /// Total unique entities (all types).
    pub fn count(&self) -> usize {
        self.persons.len()
    }

    /// Count by entity type.
    pub fn count_by_type(&self) -> (usize, usize, usize, usize) {
        let mut people = 0;
        let mut business = 0;
        let mut automated = 0;
        let mut unknown = 0;
        for p in self.persons.values() {
            match p.entity_type {
                EntityType::Person => people += 1,
                EntityType::Business => business += 1,
                EntityType::Automated => automated += 1,
                EntityType::Unknown => unknown += 1,
            }
        }
        (people, business, automated, unknown)
    }

    /// Get only real people (filtered).
    pub fn real_people_sorted(&self) -> Vec<&Person> {
        let mut people: Vec<&Person> = self
            .persons
            .values()
            .filter(|p| p.entity_type == EntityType::Person)
            .collect();
        people.sort_by(|a, b| b.interaction_count.cmp(&a.interaction_count));
        people
    }

    /// Export the full graph as JSON for the app.
    pub fn to_json(&self) -> serde_json::Value {
        let persons: Vec<serde_json::Value> = self
            .all_persons_sorted()
            .iter()
            .map(|p| {
                serde_json::json!({
                    "id": p.id,
                    "name": p.display_name,
                    "entity_type": p.entity_type,
                    "identifiers": p.identifiers,
                    "relationship": p.relationship,
                    "first_seen": p.first_seen,
                    "last_seen": p.last_seen,
                    "interactions": p.interaction_count,
                    "platforms": p.platforms,
                    "notes": p.notes,
                    "timeline": p.timeline,
                })
            })
            .collect();

        let (people, business, automated, unknown) = self.count_by_type();
        serde_json::json!({
            "total_entities": self.count(),
            "real_people": people,
            "businesses": business,
            "automated": automated,
            "unknown": unknown,
            "persons": persons,
        })
    }
}

impl Default for PeopleGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Known business/automated sender domains — NOT real people.
const AUTOMATED_DOMAINS: &[&str] = &[
    "amazon.com",
    "amazon.co",
    "facebook.com",
    "facebookmail.com",
    "fb.com",
    "google.com",
    "youtube.com",
    "gmail.com", // gmail.com is personal BUT noreply@gmail is not
    "twitter.com",
    "x.com",
    "reddit.com",
    "instagram.com",
    "snapchat.com",
    "netflix.com",
    "spotify.com",
    "apple.com",
    "icloud.com",
    "microsoft.com",
    "outlook.com",
    "live.com", // live.com can be personal (starfleet.command@live.com)
    "linkedin.com",
    "pinterest.com",
    "tiktok.com",
    "discord.com",
    "paypal.com",
    "venmo.com",
    "cashapp.com",
    "capitalone.com",
    "chase.com",
    "bankofamerica.com",
    "wellsfargo.com",
    "becu.org",
    "dominos.com",
    "doordash.com",
    "ubereats.com",
    "grubhub.com",
    "walmart.com",
    "target.com",
    "bestbuy.com",
    "newegg.com",
    "ebay.com",
    "homedepot.com",
    "lowes.com",
    "hottopic.com",
    "zumiez.com",
    "att.com",
    "tmobile.com",
    "verizon.com",
    "comcast.com",
    "xfinity.com",
    "steam.com",
    "steampowered.com",
    "epicgames.com",
    "playstation.com",
    "xbox.com",
    "github.com",
    "gitlab.com",
    "bitbucket.org",
    "heroku.com",
    "vercel.com",
    "slack.com",
    "notion.so",
    "figma.com",
    "canva.com",
    "instructables.com",
    "hackaday.com",
    "makerbot.com",
    "shapeways.com",
    "thrillist.com",
    "jackthreads.com",
    "touchofmodern.com",
    "quora.com",
    "medium.com",
    "substack.com",
    "nextdoor.com",
    "yelp.com",
    "zillow.com",
    "redfin.com",
    "nytimes.com",
    "seattletimes.com",
    "washingtonpost.com",
    "anthropic.com",
    "openai.com",
    "codingdojo.com",
    "freecodecamp.org",
    "codecademy.com",
];

/// Sender name patterns that indicate automated/business senders.
const AUTOMATED_PATTERNS: &[&str] = &[
    "noreply",
    "no-reply",
    "no_reply",
    "donotreply",
    "do-not-reply",
    "notifications",
    "notification",
    "notify",
    "alert",
    "alerts",
    "mailer-daemon",
    "postmaster",
    "bounce",
    "unsubscribe",
    "newsletter",
    "digest",
    "promo",
    "deals",
    "offers",
    "sale",
    "support",
    "help",
    "info@",
    "admin@",
    "team@",
    "hello@",
    "billing",
    "invoice",
    "receipt",
    "order",
    "shipping",
    "tracking",
    "security",
    "verify",
    "confirm",
    "welcome",
    "marketing",
    "campaign",
    "announce",
    "scheduling",
    "calendar",
    "reminder",
];

/// Entity type classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntityType {
    /// Real individual person
    Person,
    /// Business or organization
    Business,
    /// Automated sender (notifications, newsletters, bots)
    Automated,
    /// Unknown — needs manual classification
    Unknown,
}

/// Classify whether a sender is a real person, business, or automated.
pub fn classify_entity(name: &str, email_or_id: &str) -> EntityType {
    let name_lower = name.to_lowercase();
    let id_lower = email_or_id.to_lowercase();

    // Check automated patterns in the identifier
    for pattern in AUTOMATED_PATTERNS {
        if id_lower.contains(pattern) {
            return EntityType::Automated;
        }
    }

    // Check domain against known business domains
    if let Some(domain) = id_lower.split('@').nth(1) {
        for biz_domain in AUTOMATED_DOMAINS {
            if domain == *biz_domain || domain.ends_with(&format!(".{}", biz_domain)) {
                // Exception: personal emails on common domains
                // starfleet.command@live.com is a person, noreply@live.com is not
                let local = id_lower.split('@').next().unwrap_or("");
                let is_personal_pattern = !AUTOMATED_PATTERNS.iter().any(|p| local.contains(p))
                    && local.len() > 3
                    && !local.chars().all(|c| c.is_ascii_digit());

                if is_personal_pattern && is_personal_domain(domain) {
                    return EntityType::Person;
                }
                return EntityType::Business;
            }
        }
    }

    // Check if name looks like a business (no space = likely not a person name)
    if !name.is_empty() {
        // Single word names that are capitalized brands
        if !name.contains(' ') && name.len() > 2 {
            let first_char_upper = name.chars().next().map_or(false, |c| c.is_uppercase());
            // Could be a brand OR a single first name. Mark as Unknown.
            if first_char_upper && name_lower != "facebook" {
                return EntityType::Unknown;
            }
        }

        // Names with typical business suffixes
        if name_lower.contains("inc")
            || name_lower.contains("llc")
            || name_lower.contains("corp")
            || name_lower.contains("ltd")
            || name_lower.contains(" team")
            || name_lower.contains("support")
        {
            return EntityType::Business;
        }
    }

    // If we have an email on a personal domain with a real-looking local part, it's likely a person
    if email_or_id.contains('@') {
        let domain = email_or_id.split('@').nth(1).unwrap_or("");
        if is_personal_domain(domain) {
            return EntityType::Person;
        }
    }

    // Default: if we have a name with spaces (first + last), likely a person
    if name.contains(' ') && name.split_whitespace().count() >= 2 {
        return EntityType::Person;
    }

    EntityType::Unknown
}

/// Domains where individual people have accounts (vs corporate sender domains).
fn is_personal_domain(domain: &str) -> bool {
    matches!(
        domain,
        "gmail.com"
            | "yahoo.com"
            | "hotmail.com"
            | "live.com"
            | "outlook.com"
            | "aol.com"
            | "icloud.com"
            | "me.com"
            | "protonmail.com"
            | "proton.me"
            | "mail.com"
            | "ymail.com"
            | "comcast.net"
            | "msn.com"
            | "peoplepc.com"
            | "dslnorthwest.net"
            | "frontier.com"
    )
}

/// Classify what type of identifier a string is.
fn classify_identifier(value: &str) -> IdentifierType {
    if value.contains('@') {
        IdentifierType::Email
    } else if value.starts_with('+')
        && value.len() > 10
        && value[1..].chars().all(|c| c.is_ascii_digit())
    {
        IdentifierType::Phone
    } else if value.chars().all(|c| c.is_ascii_digit()) && value.len() >= 10 {
        IdentifierType::Phone
    } else {
        IdentifierType::Name
    }
}

/// Known manual merges — identifiers that belong to the same person.
/// These come from user corrections and verified data.
pub fn apply_known_merges(graph: &mut PeopleGraph) {
    // Eric Hemmen: Facebook name + Gmail address
    let eric_fb = graph
        .find_by_identifier("Eric Hemmen")
        .map(|p| p.id.clone());
    if let Some(ref eric_id) = eric_fb {
        graph.link_identifier(
            eric_id,
            PersonIdentifier {
                id_type: IdentifierType::Email,
                value: "starfleet.command@live.com".to_string(),
                platform: "gmail".to_string(),
            },
        );
    }

    // Audrey Cunningham: Facebook name + Gmail address
    let audrey_fb = graph
        .find_by_identifier("Audrey Victoria Cunningham")
        .map(|p| p.id.clone());
    if let Some(ref audrey_id) = audrey_fb {
        graph.link_identifier(
            audrey_id,
            PersonIdentifier {
                id_type: IdentifierType::Email,
                value: "pkeeper13@gmail.com".to_string(),
                platform: "gmail".to_string(),
            },
        );
    }

    // Hayden Muir: Facebook friend + Alteza coworker + iMessage
    let hayden_fb = graph
        .find_by_identifier("Hayden Muir")
        .map(|p| p.id.clone());
    if let Some(ref hayden_id) = hayden_fb {
        graph.link_identifier(
            hayden_id,
            PersonIdentifier {
                id_type: IdentifierType::Phone,
                value: "+12062515101".to_string(),
                platform: "imessage".to_string(),
            },
        );
    }

    // Jenny Marie Lieu
    let jenny_fb = graph.find_by_identifier("Jenny Lieu").map(|p| p.id.clone());
    if let Some(ref jenny_id) = jenny_fb {
        graph.link_identifier(
            jenny_id,
            PersonIdentifier {
                id_type: IdentifierType::Phone,
                value: "+12063534877".to_string(),
                platform: "imessage".to_string(),
            },
        );
        if let Some(p) = graph.persons.get_mut(jenny_id) {
            p.relationship = Some("Partner since April 2017".to_string());
            p.notes = Some("Jenny Marie Lieu. Vietnamese/Cantonese. Met at Staples early 2017. First date April 14-15, 2017.".to_string());
        }
    }

    // Natalie Cunningham: Facebook + Gmail
    let natalie = graph
        .find_by_identifier("Natalie Cunningham")
        .map(|p| p.id.clone());
    if let Some(ref id) = natalie {
        graph.link_identifier(
            id,
            PersonIdentifier {
                id_type: IdentifierType::Email,
                value: "nerdlypanda@gmail.com".into(),
                platform: "gmail".into(),
            },
        );
        if let Some(p) = graph.persons.get_mut(id) {
            p.relationship = Some("Ex-girlfriend (high school)".into());
            p.notes = Some("Natalie Cunningham. Met at HOME program co-op. Dated in high school (~2012). Calendar entry 'Asked Natalie out'.".into());
        }
    }

    // Alyssa Fung + Jennine Fung: siblings, both HOME program
    let alyssa = graph
        .find_by_identifier("Alyssa Fung")
        .map(|p| p.id.clone());
    if let Some(ref id) = alyssa {
        graph.link_identifier(
            id,
            PersonIdentifier {
                id_type: IdentifierType::Email,
                value: "aof427@gmail.com".into(),
                platform: "gmail".into(),
            },
        );
    }

    let jennine = graph
        .find_by_identifier("Jennine Fung")
        .map(|p| p.id.clone());
    if let Some(ref id) = jennine {
        graph.link_identifier(
            id,
            PersonIdentifier {
                id_type: IdentifierType::Email,
                value: "jwf619@gmail.com".into(),
                platform: "gmail".into(),
            },
        );
    }

    // Levi Sweeney: 3 email addresses
    let levi = graph
        .find_by_identifier("levi sweeney")
        .map(|p| p.id.clone())
        .or_else(|| {
            graph
                .find_by_identifier("Levi Sweeney")
                .map(|p| p.id.clone())
        });
    if let Some(ref id) = levi {
        for email in &[
            "bookworm11_11@yahoo.com",
            "levi.a.sweeney@gmail.com",
            "literaturebug157@gmail.com",
            "levi.sweeney18@gmail.com",
        ] {
            graph.link_identifier(
                id,
                PersonIdentifier {
                    id_type: IdentifierType::Email,
                    value: email.to_string(),
                    platform: "gmail".into(),
                },
            );
        }
    }

    // Jeffery Towne: 3 email addresses (Nick's dad)
    let jeff = graph
        .find_by_identifier("Jeffery Towne")
        .map(|p| p.id.clone());
    if let Some(ref id) = jeff {
        for email in &[
            "jtowne@dslnorthwest.net",
            "jefferytowne@me.com",
            "jeff.towne_693@comcast.net",
        ] {
            graph.link_identifier(
                id,
                PersonIdentifier {
                    id_type: IdentifierType::Email,
                    value: email.to_string(),
                    platform: "gmail".into(),
                },
            );
        }
        if let Some(p) = graph.persons.get_mut(id) {
            p.relationship = Some("Father".into());
        }
    }

    // Lucy Towne: Nick's mom
    let lucy = graph.find_by_identifier("Lucy Towne").map(|p| p.id.clone());
    if let Some(ref id) = lucy {
        graph.link_identifier(
            id,
            PersonIdentifier {
                id_type: IdentifierType::Email,
                value: "littleredcaboose@dslnorthwest.net".into(),
                platform: "gmail".into(),
            },
        );
        if let Some(p) = graph.persons.get_mut(id) {
            p.relationship = Some("Mother".into());
        }
    }

    // Sydney Towne: Nick's sister
    let sydney = graph
        .find_by_identifier("Sydney Towne")
        .map(|p| p.id.clone());
    if let Some(ref id) = sydney {
        graph.link_identifier(
            id,
            PersonIdentifier {
                id_type: IdentifierType::Email,
                value: "towne.sydney@gmail.com".into(),
                platform: "gmail".into(),
            },
        );
        if let Some(p) = graph.persons.get_mut(id) {
            p.relationship = Some("Older sister".into());
        }
    }

    // Jeffrey Hemmen: Eric's dad
    let jeff_h = graph
        .find_by_identifier("Jeffrey Hemmen")
        .map(|p| p.id.clone());
    if let Some(ref id) = jeff_h {
        graph.link_identifier(
            id,
            PersonIdentifier {
                id_type: IdentifierType::Email,
                value: "hemjef@hotmail.com".into(),
                platform: "gmail".into(),
            },
        );
        if let Some(p) = graph.persons.get_mut(id) {
            p.notes = Some("Eric Hemmen's father".into());
        }
    }

    info!("applied known merges");
}
