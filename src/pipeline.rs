//! Ingest pipeline — orchestrates the full flow from discovery to output.
//!
//! discover → extract → deduplicate → output
//!
//! Neuron extracts and deduplicates records. Storage is the consumer's
//! responsibility — EVA stores to MemPalace HNSW, others can store
//! wherever they want. Neuron just produces clean CommonRecords.

use crate::adapters::SourceAdapter;
use crate::common::CommonRecord;
use std::collections::HashSet;
use std::path::Path;
use tracing::{info, warn};

/// Report from an ingest run.
#[derive(Debug)]
pub struct IngestReport {
    /// Adapter name
    pub adapter: String,
    /// Platform
    pub platform: String,
    /// Source path
    pub path: String,
    /// Records extracted
    pub extracted: usize,
    /// Records after deduplication
    pub deduplicated: usize,
    /// Errors encountered
    pub errors: usize,
}

/// Extract and deduplicate records from a single source.
/// Returns clean CommonRecords ready for whatever storage the consumer uses.
pub fn extract_source(
    adapter: &dyn SourceAdapter,
    path: &Path,
    seen_ids: &mut HashSet<String>,
) -> (Vec<CommonRecord>, IngestReport) {
    info!(
        adapter = adapter.name(),
        path = %path.display(),
        "starting extraction"
    );

    // Extract records
    let records = match adapter.extract_from_file(path) {
        Ok(r) => r,
        Err(e) => {
            warn!(adapter = adapter.name(), error = %e, "extraction failed");
            return (
                vec![],
                IngestReport {
                    adapter: adapter.name().to_string(),
                    platform: adapter.platform().to_string(),
                    path: path.to_string_lossy().to_string(),
                    extracted: 0,
                    deduplicated: 0,
                    errors: 1,
                },
            );
        }
    };

    let total = records.len();

    // Deduplicate
    let mut unique = Vec::new();
    for record in records {
        let id = record.id();
        if seen_ids.contains(&id) {
            continue;
        }
        seen_ids.insert(id);
        unique.push(record);
    }

    let deduped = unique.len();

    info!(
        adapter = adapter.name(),
        extracted = total,
        deduplicated = deduped,
        "extraction complete"
    );

    (
        unique,
        IngestReport {
            adapter: adapter.name().to_string(),
            platform: adapter.platform().to_string(),
            path: path.to_string_lossy().to_string(),
            extracted: total,
            deduplicated: deduped,
            errors: 0,
        },
    )
}

/// Export records to JSONL file (portable output format).
pub fn export_jsonl(records: &[CommonRecord], path: &Path) -> Result<usize, String> {
    use std::io::Write;
    let file = std::fs::File::create(path)
        .map_err(|e| format!("Failed to create {}: {}", path.display(), e))?;
    let mut writer = std::io::BufWriter::new(file);

    let mut count = 0;
    for record in records {
        let json = serde_json::to_string(record)
            .map_err(|e| format!("Failed to serialize record: {}", e))?;
        writeln!(writer, "{}", json).map_err(|e| format!("Failed to write: {}", e))?;
        count += 1;
    }

    info!(records = count, path = %path.display(), "exported to JSONL");
    Ok(count)
}
