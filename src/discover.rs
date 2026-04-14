//! Auto-discovery — scan the local machine for known data sources.
//!
//! Checks common locations for platform exports, browser databases,
//! message archives, and other data EVA can ingest.

use crate::adapters::SourceAdapter;
use std::path::PathBuf;
use tracing::info;

/// A discovered data source on the local machine.
#[derive(Debug)]
pub struct DiscoveredSource {
    /// Which adapter can handle this
    pub adapter_name: String,
    /// Platform identifier
    pub platform: String,
    /// Path to the data
    pub path: PathBuf,
    /// Estimated record count (if available)
    pub estimated_records: Option<usize>,
}

/// Scan the local machine for all known data sources.
pub fn discover_all(adapters: &[Box<dyn SourceAdapter>]) -> Vec<DiscoveredSource> {
    let mut found = Vec::new();

    for adapter in adapters {
        let paths = adapter.discover_local();
        for path in paths {
            let estimate = adapter.estimate_count(&path);
            info!(
                adapter = adapter.name(),
                path = %path.display(),
                estimate = ?estimate,
                "discovered data source"
            );
            found.push(DiscoveredSource {
                adapter_name: adapter.name().to_string(),
                platform: adapter.platform().to_string(),
                path,
                estimated_records: estimate,
            });
        }
    }

    info!(total = found.len(), "discovery scan complete");
    found
}
