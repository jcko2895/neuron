//! Test all new adapters against real data.

use neuron::adapters::SourceAdapter;
use std::path::PathBuf;
use std::time::Instant;

fn test_adapter(adapter: &dyn SourceAdapter, path: &std::path::Path) {
    println!("\n{}", "=".repeat(60));
    println!("  {} — {}", adapter.name(), path.display());
    println!("{}", "=".repeat(60));

    if !adapter.can_handle_file(path) {
        println!("  SKIP: can_handle_file returned false");
        return;
    }

    let start = Instant::now();
    match adapter.extract_from_file(path) {
        Ok(records) => {
            let elapsed = start.elapsed();
            let rate = if elapsed.as_secs_f64() > 0.0 {
                records.len() as f64 / elapsed.as_secs_f64()
            } else {
                0.0
            };
            println!("  Records: {}", records.len());
            println!("  Time:    {:.2?}", elapsed);
            println!("  Rate:    {:.0} records/sec", rate);

            // Show first 3 records
            for (i, r) in records.iter().take(3).enumerate() {
                let content_preview = if r.content.len() > 80 {
                    format!("{}...", &r.content[..80])
                } else {
                    r.content.clone()
                };
                println!("  [{}] {} | ts={} | actor={}",
                    i,
                    content_preview,
                    r.timestamp.as_deref().unwrap_or("none"),
                    r.actor.as_deref().unwrap_or("none"),
                );
            }
        }
        Err(e) => {
            println!("  ERROR: {}", e);
        }
    }
}

fn main() {
    println!("=== Neuron Adapter Test Suite ===\n");

    // Instagram
    let instagram = neuron::adapters::instagram::InstagramAdapter::new("Nick");
    test_adapter(&instagram, &PathBuf::from("D:/EVA/SUBSTRATE/data/raw/instagram"));

    // iMessage (individual files)
    let imessage = neuron::adapters::imessage::IMessageAdapter::new("Nick");
    test_adapter(&imessage, &PathBuf::from("D:/EVA/SUBSTRATE/data/iphone_iphone6_2015_messages.jsonl"));
    test_adapter(&imessage, &PathBuf::from("D:/EVA/SUBSTRATE/data/iphone_iphone6s_2016_messages.jsonl"));

    // Google Takeout
    let google = neuron::adapters::google_takeout::GoogleTakeoutAdapter::new("Nick");
    test_adapter(&google, &PathBuf::from("G:/staging/google-takeout/Takeout"));

    // Breakdown by source_type
    {
        let path = PathBuf::from("G:/staging/google-takeout/Takeout");
        if let Ok(records) = google.extract_from_file(&path) {
            let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
            for r in &records {
                *counts.entry(r.source_type.clone()).or_insert(0) += 1;
            }
            println!("\n  Google Takeout breakdown:");
            let mut sorted: Vec<_> = counts.into_iter().collect();
            sorted.sort_by(|a, b| b.1.cmp(&a.1));
            for (source_type, count) in &sorted {
                println!("    {}: {}", source_type, count);
            }
        }
    }

    println!("\n=== Done ===");
}
