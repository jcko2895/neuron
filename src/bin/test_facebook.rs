//! Test the Facebook adapter against raw Takeout data.

use neuron::adapters::SourceAdapter;
use neuron::adapters::facebook::FacebookAdapter;
use neuron::pipeline;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!("==============================================");
    println!("  Neuron: Facebook Takeout");
    println!("==============================================");

    let adapter = FacebookAdapter::new("Nicholas Wilson Towne");
    let source_path = PathBuf::from("D:/EVA/SUBSTRATE/data/raw/facebook_full");

    if !adapter.can_handle_file(&source_path) {
        println!("  No Facebook data found");
        return;
    }

    let start = Instant::now();
    let mut seen = HashSet::new();
    let (records, report) = pipeline::extract_source(&adapter, &source_path, &mut seen);

    println!(
        "  Extracted: {} | Deduped: {} | Time: {:?}",
        report.extracted,
        report.deduplicated,
        start.elapsed()
    );
    println!("  Neurons ready: {}", records.len());
}
