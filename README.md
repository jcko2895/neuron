# Neuron

Turn your digital life into AI-ready data. Every platform. Full provenance.

Neuron extracts your data from every platform you use — Facebook, Gmail, Instagram, iMessage, browser history, and more — and turns it into structured records with full provenance tracking. Every record traces back to its original source file with a SHA256 content hash.

## Why

Your data is scattered across dozens of platforms in dozens of formats. No AI can use it because it's siloed, unstructured, and untraceable. Neuron fixes that:

- **One tool, every platform** — adapters for Facebook, Gmail, Instagram, Snapchat, browser history, and more
- **Full provenance** — every record has: source file path, content hash, trust level, timestamp, platform, actor
- **Two paths per platform** — file import (parse export files) or API connector (OAuth + live sync)
- **Entity resolution** — the same person across platforms gets merged into one identity
- **Trust levels** — raw exports are "Primary," AI-processed data is "Secondary," user claims are "UserClaim"
- **No storage opinion** — Neuron extracts and deduplicates. You store wherever you want.

## Install

```bash
cargo install neuron
```

Or build from source:

```bash
git clone https://github.com/jcko2895/neuron
cd neuron
cargo build --release
```

## Usage

```rust
use neuron::adapters::facebook::FacebookAdapter;
use neuron::adapters::SourceAdapter;
use neuron::pipeline;
use std::collections::HashSet;
use std::path::PathBuf;

let adapter = FacebookAdapter::new("Your Name");
let path = PathBuf::from("/path/to/facebook/takeout");

let mut seen = HashSet::new();
let (records, report) = pipeline::extract_source(&adapter, &path, &mut seen);

// records: Vec<CommonRecord> — ready for any storage backend
// Every record has: content, timestamp, actor, source_file, content_hash, trust_level
```

## Adapters

| Platform | File Import | API Connector | Status |
|----------|-------------|---------------|--------|
| Facebook | ✅ Takeout JSON | Planned | Working |
| Gmail | ✅ .eml files | Planned | Working |
| Instagram | Planned | Planned | — |
| Snapchat | Planned | Planned | — |
| iMessage | Planned | — | — |
| Browser (Chrome/Edge/Firefox) | Planned | — | — |
| Google Takeout (YouTube, Calendar, Location, Contacts) | Planned | Planned | — |
| Twitter/X | Planned | Planned | — |
| Discord | Planned | Planned | — |
| Apple Health | Planned | Planned | — |

## CommonRecord

Every adapter outputs the same format:

```rust
pub struct CommonRecord {
    pub content: String,           // The actual text
    pub timestamp: Option<String>, // When it was created (ISO 8601)
    pub actor: Option<String>,     // Who created it
    pub is_user: bool,             // Is this the user or someone else?
    pub source_file: String,       // Path to the raw source file
    pub source_type: String,       // "facebook_takeout_raw", "gmail_eml", etc.
    pub trust_level: TrustLevel,   // Primary, Secondary, or UserClaim
    pub content_hash: String,      // SHA256 of original content
    pub platform: String,          // "facebook", "gmail", etc.
    pub thread_id: Option<String>, // Conversation grouping
    pub thread_name: Option<String>,
    pub account: Option<AccountContext>,
    pub metadata: serde_json::Value,
}
```

## Entity Resolution

Neuron merges the same person across platforms:

```rust
use neuron::entity::PeopleGraph;

let mut graph = PeopleGraph::new();
graph.process_records(&records);

// "Eric Hemmen" on Facebook = "starfleet.command@live.com" in Gmail
// → One Person entity with both identifiers
```

## Trust Levels

- **Primary** — raw platform export, unmodified. SHA256 verifiable.
- **Secondary** — AI-processed or derived data. Something may have been altered.
- **UserClaim** — user stated this in conversation. May be accurate, may be false memory. Must be cross-referenced.

## Philosophy

Your memory is not trustworthy. Facts are. Neuron treats every data source — including the user — as a claim that needs verification. A "fact" is a claim with multiple independent sources confirming it.

## License

MIT
