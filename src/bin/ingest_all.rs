//! Ingest all available data sources and export to JSONL.
//!
//! Runs every working adapter against known data locations,
//! deduplicates, and exports to a single JSONL file.

use neuron::adapters::SourceAdapter;
use neuron::pipeline;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

struct Source {
    name: &'static str,
    adapter: Box<dyn SourceAdapter>,
    paths: Vec<PathBuf>,
}

fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter("neuron=info")
        .init();

    println!("=== Neuron Full Ingest ===\n");
    let start = Instant::now();

    let sources: Vec<Source> = vec![
        Source {
            name: "Facebook",
            adapter: Box::new(neuron::adapters::facebook::FacebookAdapter::new("Nick")),
            paths: vec![PathBuf::from("D:/EVA/SUBSTRATE/data/raw/facebook_full")],
        },
        Source {
            name: "Gmail",
            adapter: Box::new(neuron::adapters::gmail::GmailAdapter::new("nicktowne2895@gmail.com", "Nick")),
            paths: vec![PathBuf::from("D:/UserData/Documents/Emails/Gmail/raw/Takeout")],
        },
        Source {
            name: "Instagram",
            adapter: Box::new(neuron::adapters::instagram::InstagramAdapter::new("Nick")),
            paths: vec![PathBuf::from("D:/EVA/SUBSTRATE/data/raw/instagram")],
        },
        Source {
            name: "iMessage (iPhone 6 2015)",
            adapter: Box::new(neuron::adapters::imessage::IMessageAdapter::new("Nick")),
            paths: vec![
                PathBuf::from("D:/EVA/SUBSTRATE/data/iphone_iphone6_2015_messages.jsonl"),
            ],
        },
        Source {
            name: "iMessage (iPhone 6s 2016)",
            adapter: Box::new(neuron::adapters::imessage::IMessageAdapter::new("Nick")),
            paths: vec![
                PathBuf::from("D:/EVA/SUBSTRATE/data/iphone_iphone6s_2016_messages.jsonl"),
            ],
        },
        Source {
            name: "iMessage (MacBook chat.db)",
            adapter: Box::new(neuron::adapters::imessage_db::IMessageDbAdapter::new("Nick")),
            paths: vec![PathBuf::from("D:/EVA/SUBSTRATE/data/raw/macbook/chat.db")],
        },
        Source {
            name: "Spotify",
            adapter: Box::new(neuron::adapters::spotify::SpotifyAdapter::new("Nick")),
            paths: vec![PathBuf::from("D:/EVA/SUBSTRATE/data/raw/spotify")],
        },
        Source {
            name: "Google Takeout",
            adapter: Box::new(neuron::adapters::google_takeout::GoogleTakeoutAdapter::new("Nick")),
            paths: vec![PathBuf::from("G:/staging/google-takeout/Takeout")],
        },
        Source {
            name: "Edge Browser",
            adapter: Box::new(neuron::adapters::browser::BrowserHistoryAdapter::new(
                neuron::adapters::browser::BrowserKind::Edge,
            )),
            paths: vec![PathBuf::from("C:/Users/Nick/AppData/Local/Microsoft/Edge/User Data/Default/History")],
        },
        Source {
            name: "Safari (MacBook)",
            adapter: Box::new(neuron::adapters::browser::BrowserHistoryAdapter::new(
                neuron::adapters::browser::BrowserKind::Safari,
            )),
            paths: vec![PathBuf::from("D:/EVA/SUBSTRATE/data/raw/macbook/safari_history.db")],
        },
        Source {
            name: "Apple Photos (metadata)",
            adapter: Box::new(neuron::adapters::apple_photos::ApplePhotosAdapter::new("Nick")),
            paths: vec![PathBuf::from("D:/EVA/SUBSTRATE/data/raw/macbook/photos.sqlite")],
        },
        Source {
            name: "Apple Notes",
            adapter: Box::new(neuron::adapters::apple_notes::AppleNotesAdapter::new("Nick")),
            paths: vec![PathBuf::from("D:/EVA/SUBSTRATE/data/raw/macbook/notes.sqlite")],
        },
        Source {
            name: "Apple Contacts",
            adapter: Box::new(neuron::adapters::apple_contacts::AppleContactsAdapter::new("Nick")),
            paths: vec![PathBuf::from("D:/EVA/SUBSTRATE/data/raw/macbook/contacts")],
        },
        Source {
            name: "ChatGPT",
            adapter: Box::new(neuron::adapters::chatgpt::ChatGptAdapter::new("Nick")),
            paths: vec![PathBuf::from("G:/archive/downloads/ChatGPT_Export-2025-09-06.zip")],
        },
        Source {
            name: "Facebook Friends",
            adapter: Box::new(neuron::adapters::facebook_friends::FacebookFriendsAdapter::new("Nick")),
            paths: vec![PathBuf::from("D:/EVA/SUBSTRATE/data/raw/facebook3/connections/friends")],
        },
    ];

    let mut seen = HashSet::new();
    let mut all_records = Vec::new();
    let mut reports = Vec::new();

    for source in &sources {
        for path in &source.paths {
            if !path.exists() {
                println!("  SKIP {} — path not found: {}", source.name, path.display());
                continue;
            }

            let src_start = Instant::now();
            let (records, report) = pipeline::extract_source(source.adapter.as_ref(), path, &mut seen);
            let elapsed = src_start.elapsed();

            let rate = if elapsed.as_secs_f64() > 0.0 {
                records.len() as f64 / elapsed.as_secs_f64()
            } else {
                0.0
            };

            println!(
                "  {:30} {:>8} records  ({:>8} extracted, {:>8} deduped)  {:.2?}  {:.0}/sec",
                source.name,
                records.len(),
                report.extracted,
                report.deduplicated,
                elapsed,
                rate,
            );

            all_records.extend(records);
            reports.push(report);
        }
    }

    let total_elapsed = start.elapsed();

    println!("\n{}", "=".repeat(70));
    println!("  Total records: {}", all_records.len());
    println!("  Total time:    {:.2?}", total_elapsed);
    println!("{}", "=".repeat(70));

    // Export to JSONL
    let output_path = PathBuf::from("D:/EVA/SUBSTRATE/data/neuron_full_export.jsonl");
    match pipeline::export_jsonl(&all_records, &output_path) {
        Ok(count) => println!("\n  Exported {} records to {}", count, output_path.display()),
        Err(e) => println!("\n  ERROR exporting: {}", e),
    }

    println!("\n=== Done ===");
}
