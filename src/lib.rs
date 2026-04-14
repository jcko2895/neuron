//! Neuron — turn your digital life into AI-ready data with full provenance.
//!
//! Every piece of data gets:
//! - A SHA256 content hash (verify nothing was altered)
//! - A source file path (trace back to the original)
//! - A trust level (primary = raw export, secondary = AI-processed, user_claim = user said it)
//! - A platform tag (facebook, imessage, chrome, etc.)
//! - A timestamp from the source (not when we ingested it)
//!
//! Two paths for every platform:
//! - **API Connector**: OAuth to the platform, pull data live
//! - **File Import**: user drops an export file, Neuron parses it
//!
//! Same CommonRecord output either way. Same provenance. User chooses.
//! Storage is the consumer's responsibility — Neuron extracts, you store.

pub mod adapters;
pub mod common;
pub mod discover;
pub mod entity;
pub mod pipeline;
