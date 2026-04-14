//! Source adapters — one per platform.
//!
//! Every adapter implements the SourceAdapter trait with two paths:
//! - File import (parse local export files)
//! - API connector (OAuth + live sync)
//!
//! Same CommonRecord output either way.

use crate::common::CommonRecord;
use std::path::{Path, PathBuf};

pub mod facebook;
pub mod gmail;

/// The universal adapter interface.
///
/// Every platform (Facebook, iMessage, Chrome, etc.) implements this trait.
/// The pipeline calls these methods to discover, extract, and sync data.
pub trait SourceAdapter: Send + Sync {
    /// Human-readable name ("Facebook", "Gmail", "iMessage", etc.)
    fn name(&self) -> &str;

    /// Platform identifier ("facebook", "gmail", "imessage", etc.)
    fn platform(&self) -> &str;

    // ── File Import Path ──────────────────────────────

    /// Can this adapter handle data at the given path?
    fn can_handle_file(&self, path: &Path) -> bool;

    /// Extract records from a local file or directory.
    /// Returns raw CommonRecords with full provenance.
    fn extract_from_file(&self, path: &Path) -> Result<Vec<CommonRecord>, String>;

    // ── API Connector Path ──────────────────────────────

    /// Does this adapter support live API connection?
    fn supports_api(&self) -> bool {
        false
    }

    /// Start OAuth flow. Returns URL for user to authorize.
    fn begin_auth(&self) -> Result<String, String> {
        Err("API not supported for this adapter".into())
    }

    /// Complete OAuth with callback code. Stores tokens.
    fn complete_auth(&self, _code: &str) -> Result<(), String> {
        Err("API not supported".into())
    }

    /// Pull latest data from API (incremental — only new since last sync).
    fn sync(&self) -> Result<Vec<CommonRecord>, String> {
        Err("API not supported".into())
    }

    // ── Discovery ──────────────────────────────

    /// Auto-detect if this platform's data exists on the local machine.
    /// Returns paths to discovered data.
    fn discover_local(&self) -> Vec<PathBuf> {
        vec![]
    }

    /// Estimated record count for progress bars.
    fn estimate_count(&self, _path: &Path) -> Option<usize> {
        None
    }
}
