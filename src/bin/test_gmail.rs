//! Test the Gmail adapter against .eml Takeout data.

use neuron::adapters::SourceAdapter;
use neuron::adapters::gmail::GmailAdapter;
use neuron::entity::{self, PeopleGraph};
use neuron::pipeline;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!("==============================================");
    println!("  Neuron: Gmail Takeout");
    println!("==============================================");

    let adapter = GmailAdapter::new("NickTowne2895@gmail.com", "Nicholas Towne");
    let source_path = PathBuf::from("D:/UserData/Documents/Emails/Gmail/raw/Takeout");

    if !adapter.can_handle_file(&source_path) {
        println!("  No Gmail data found");
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

    // People graph
    let mut graph = PeopleGraph::new();
    graph.process_records(&records);
    entity::apply_known_merges(&mut graph);
    let (people, business, automated, unknown) = graph.count_by_type();
    println!(
        "  People: {} | Business: {} | Auto: {} | Unknown: {}",
        people, business, automated, unknown
    );

    println!("  Neurons ready: {}", records.len());
}
