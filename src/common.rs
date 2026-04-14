//! Common record schema — the universal format every adapter outputs.
//!
//! Every piece of data from every platform gets normalized to this schema
//! before entering MemPalace. Full provenance chain on every record.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// How much we trust this data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrustLevel {
    /// Raw platform export, unmodified. SHA256 verifiable against source file.
    Primary,
    /// AI-processed or derived. Something between the original and this record
    /// may have altered, summarized, or reinterpreted the content.
    Secondary,
    /// User stated this in conversation. May be accurate, may be false memory.
    /// Must be cross-referenced against primary sources before accepting as fact.
    UserClaim,
}

/// Context for the account this data came from.
/// The same user behaves differently across accounts — a gaming account
/// is not a personal account. EVA must preserve this context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountContext {
    /// Platform (facebook, discord, twitter, imessage, etc.)
    pub platform: String,
    /// Account identifier (username, email, phone number)
    pub account_id: String,
    /// Display name on this platform
    pub display_name: String,
    /// What kind of account this is (personal, gaming, professional, anonymous)
    pub account_type: String,
    /// How the user behaves in this context (learned over time)
    pub persona_notes: Option<String>,
}

/// A single record extracted from any data source.
/// This is the universal format — every adapter produces these.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommonRecord {
    /// The actual text content, unmodified from source.
    pub content: String,

    /// When this content was created (ISO 8601, from the source).
    /// NOT when we ingested it.
    pub timestamp: Option<String>,

    /// Who created this content (person's name, username, phone number).
    pub actor: Option<String>,

    /// Is this the user or someone else?
    pub is_user: bool,

    // ── Provenance ──────────────────────────────────
    /// Path to the raw source file on disk.
    pub source_file: String,

    /// What type of source (e.g., "facebook_takeout_raw", "imessage_backup", "edge_history_sqlite").
    pub source_type: String,

    /// How much we trust this data.
    pub trust_level: TrustLevel,

    /// SHA256 hash of the original content bytes. Used to verify nothing was altered.
    pub content_hash: String,

    /// Which platform this came from.
    pub platform: String,

    // ── Context ──────────────────────────────────
    /// Conversation/thread grouping (for messages).
    pub thread_id: Option<String>,

    /// Thread/conversation name (e.g., "boneless pizza" group chat).
    pub thread_name: Option<String>,

    /// Which account context this belongs to.
    pub account: Option<AccountContext>,

    /// Platform-specific metadata (reactions, photos, etc.)
    pub metadata: serde_json::Value,
}

impl CommonRecord {
    /// Compute a unique ID for this record based on source + content.
    /// Same content from same source = same ID (deduplication).
    pub fn id(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.source_file.as_bytes());
        hasher.update(b"\x00");
        hasher.update(self.content.as_bytes());
        hasher.update(b"\x00");
        if let Some(ts) = &self.timestamp {
            hasher.update(ts.as_bytes());
        }
        let hash = hasher.finalize();
        format!("{:x}", hash)[..24].to_string()
    }

    /// Compute SHA256 hash of the raw content for integrity verification.
    pub fn compute_content_hash(content: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Convert to MemPalace drawer metadata JSON.
    pub fn to_drawer_metadata(&self) -> serde_json::Value {
        serde_json::json!({
            "source_file": self.source_file,
            "source_type": self.source_type,
            "trust_level": self.trust_level,
            "content_hash": self.content_hash,
            "platform": self.platform,
            "timestamp": self.timestamp,
            "actor": self.actor,
            "is_user": self.is_user,
            "thread_id": self.thread_id,
            "thread_name": self.thread_name,
            "account": self.account,
            "ingested_at": now_iso(),
            "modified_by": serde_json::Value::Null,
            "extra": self.metadata,
        })
    }
}

fn now_iso() -> String {
    // Simple ISO timestamp without chrono dependency
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Basic ISO format
    format!(
        "2026-04-13T{:02}:{:02}:{:02}Z",
        (secs / 3600) % 24,
        (secs / 60) % 60,
        secs % 60
    )
}
