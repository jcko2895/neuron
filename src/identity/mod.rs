//! Identity Graph — cross-platform entity resolution.
//!
//! Resolves the same person across multiple platforms into a single identity.
//! Standalone tool: works for anyone, no user-specific code.
//!
//! # Architecture
//!
//! ```text
//! Neuron JSONL + Contacts DBs + Platform Friend Mappings
//!                     ↓
//!            Identity Resolver (rules-based)
//!                     ↓
//!              identity.db (SQLite)
//!                     ↓
//!         people.json / API / direct query
//! ```
//!
//! # Resolution Rules (universal)
//!
//! 1. Same full name (first + last) on different platforms → same person
//! 2. Different last names → ALWAYS different people, no exceptions
//! 3. First name alone → NEVER merge
//! 4. Phone number in contacts → resolves to contact's full name
//! 5. Platform username in friend mapping → resolves to display name
//! 6. Email in contacts → resolves to contact's full name
//! 7. User correction → overrides automation permanently
//! 8. Ambiguous → do NOT merge, flag for review

pub mod db;
pub mod resolver;
pub mod contacts;
pub mod platform_friends;

use serde::{Deserialize, Serialize};

/// A unique person in the identity graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Person {
    /// Unique ID (UUID or auto-increment).
    pub id: i64,
    /// The canonical display name — first + last.
    pub canonical_name: String,
    /// First name (parsed from canonical_name).
    pub first_name: String,
    /// Last name (parsed from canonical_name). Empty if unknown.
    pub last_name: String,
    /// Gender: M, F, or ? (unknown).
    pub gender: String,
    /// All known identifiers across platforms.
    pub identifiers: Vec<Identifier>,
    /// Social groups this person belongs to.
    pub groups: Vec<String>,
    /// Connections to other people.
    pub connections: Vec<Connection>,
    /// First interaction timestamp (ISO 8601).
    pub first_seen: Option<String>,
    /// Last interaction timestamp (ISO 8601).
    pub last_seen: Option<String>,
    /// Total interaction count (computed from all identifiers).
    pub interaction_count: i64,
    /// Platforms this person appears on.
    pub platforms: Vec<String>,
}

/// An identifier — a way to reach or recognize a person on a specific platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identifier {
    /// Which platform: imessage, facebook, instagram, snapchat, gmail, etc.
    pub platform: String,
    /// The actual value: phone number, username, email, display name.
    pub value: String,
    /// Type of identifier: phone, username, email, display_name.
    pub id_type: IdentifierType,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum IdentifierType {
    Phone,
    Email,
    Username,
    DisplayName,
    FullName,
}

impl std::fmt::Display for IdentifierType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Phone => write!(f, "phone"),
            Self::Email => write!(f, "email"),
            Self::Username => write!(f, "username"),
            Self::DisplayName => write!(f, "display_name"),
            Self::FullName => write!(f, "full_name"),
        }
    }
}

impl IdentifierType {
    pub fn from_str(s: &str) -> Self {
        match s {
            "phone" => Self::Phone,
            "email" => Self::Email,
            "username" => Self::Username,
            "display_name" => Self::DisplayName,
            "full_name" => Self::FullName,
            _ => Self::DisplayName,
        }
    }
}

/// A relationship between two people.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    /// The other person's ID.
    pub person_id: i64,
    /// Type: sibling, parent, child, partner, friend, coworker, etc.
    pub connection_type: String,
    /// Optional note.
    pub note: String,
}

/// A user correction that overrides automated resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Correction {
    pub id: i64,
    /// Which person this correction applies to.
    pub person_id: i64,
    /// What field was corrected: canonical_name, gender, group, relationship_type, etc.
    pub field: String,
    /// Old value (what the system had).
    pub old_value: String,
    /// New value (what the user said).
    pub new_value: String,
    /// When the correction was made.
    pub timestamp: String,
    /// Who made the correction: "user", "agent", etc.
    pub source: String,
}

/// Parse a name into first and last components.
/// Returns (first_name, last_name). Last name is empty if only one word.
pub fn parse_name(name: &str) -> (String, String) {
    let parts: Vec<&str> = name.trim().split_whitespace().collect();
    match parts.len() {
        0 => (String::new(), String::new()),
        1 => (parts[0].to_string(), String::new()),
        _ => {
            let first = parts[0].to_string();
            let last = parts[1..].join(" ");
            (first, last)
        }
    }
}

/// Normalize a phone number to digits only, last 10 digits.
pub fn normalize_phone(phone: &str) -> Option<String> {
    let digits: String = phone.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() >= 10 {
        Some(digits[digits.len() - 10..].to_string())
    } else {
        None
    }
}

/// Normalize an email to lowercase, trimmed.
pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}
