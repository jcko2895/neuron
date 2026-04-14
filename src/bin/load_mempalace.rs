//! Load Neuron JSONL export into MemPalace via Ollama embeddings.
//!
//! Reads neuron_full_export.jsonl, batches text through nomic-embed-text,
//! and inserts drawers into a MemPalace SQLite store for persistence.
//!
//! Estimated: ~100 embeddings/sec via Ollama batch API → 1.28M records in ~3.5 hours.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::Instant;

const BATCH_SIZE: usize = 50;
const OLLAMA_URL: &str = "http://localhost:11434/api/embed";
const MODEL: &str = "nomic-embed-text";

#[derive(Debug, Deserialize)]
struct CommonRecord {
    content: String,
    timestamp: Option<String>,
    actor: Option<String>,
    is_user: bool,
    source_file: String,
    source_type: String,
    platform: String,
    thread_id: Option<String>,
    thread_name: Option<String>,
    content_hash: String,
    metadata: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct EmbedRequest {
    model: String,
    input: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct MemPalaceDrawer {
    id: String,
    content: String,
    wing: String,
    room: String,
    source: String,
    metadata: serde_json::Value,
    embedding: Vec<f32>,
}

fn platform_to_wing(platform: &str) -> &str {
    match platform {
        "facebook" | "instagram" | "imessage" | "snapchat" => "social",
        "gmail" => "email",
        "spotify" | "apple_music" => "music",
        "youtube" | "google_activity" => "activity",
        "google_chrome" | "edge" | "safari" | "firefox" => "browsing",
        "chatgpt" | "claude" | "codex" | "gemini" => "ai_conversations",
        "google_calendar" => "calendar",
        "google_contacts" | "apple_contacts" => "people",
        "apple_photos" => "photos",
        "apple_notes" => "notes",
        _ => "general",
    }
}

fn classify_room(record: &CommonRecord) -> String {
    // Use thread_name if available, otherwise source_type
    if let Some(thread) = &record.thread_name {
        if !thread.is_empty() {
            return thread.clone();
        }
    }
    record.source_type.clone()
}

fn embed_batch(client: &ureq::Agent, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
    let request = EmbedRequest {
        model: MODEL.to_string(),
        input: texts.to_vec(),
    };

    let body = serde_json::to_string(&request).map_err(|e| e.to_string())?;

    let response = client
        .post(OLLAMA_URL)
        .set("Content-Type", "application/json")
        .send_string(&body)
        .map_err(|e| format!("Ollama request failed: {}", e))?;

    let resp: EmbedResponse = response
        .into_json()
        .map_err(|e| format!("Failed to parse Ollama response: {}", e))?;

    Ok(resp.embeddings)
}

fn main() {
    let input_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "D:/EVA/SUBSTRATE/data/neuron_full_export.jsonl".to_string());
    let output_path = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "D:/EVA/SUBSTRATE/data/mempalace_drawers.jsonl".to_string());

    // Resume support: check how many we already processed
    let already_done = if std::path::Path::new(&output_path).exists() {
        let file = std::fs::File::open(&output_path).unwrap();
        std::io::BufReader::new(file).lines().count()
    } else {
        0
    };

    println!("=== Neuron → MemPalace Loader ===");
    println!("Input:  {}", input_path);
    println!("Output: {}", output_path);
    if already_done > 0 {
        println!("Resuming from record {}", already_done);
    }

    let input = std::fs::File::open(&input_path)
        .expect("Failed to open input JSONL");
    let reader = std::io::BufReader::new(input);

    let output = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&output_path)
        .expect("Failed to open output file");
    let mut writer = std::io::BufWriter::new(output);

    let client = ureq::agent();

    let mut batch_texts: Vec<String> = Vec::new();
    let mut batch_records: Vec<CommonRecord> = Vec::new();
    let mut total = 0usize;
    let mut embedded = 0usize;
    let mut skipped = 0usize;
    let start = Instant::now();
    let mut last_report = Instant::now();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line.trim().is_empty() { continue; }

        total += 1;

        // Skip already-processed records for resume
        if total <= already_done {
            continue;
        }

        let record: CommonRecord = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(_) => { skipped += 1; continue; }
        };

        // Truncate very long content for embedding (nomic-embed-text has ~8K token limit)
        let text = if record.content.len() > 2000 {
            let mut end = 2000;
            while !record.content.is_char_boundary(end) { end -= 1; }
            record.content[..end].to_string()
        } else {
            record.content.clone()
        };

        batch_texts.push(text);
        batch_records.push(record);

        if batch_texts.len() >= BATCH_SIZE {
            match embed_batch(&client, &batch_texts) {
                Ok(embeddings) => {
                    for (record, embedding) in batch_records.drain(..).zip(embeddings.into_iter()) {
                        let drawer = MemPalaceDrawer {
                            id: record.content_hash.clone(),
                            content: record.content.clone(),
                            wing: platform_to_wing(&record.platform).to_string(),
                            room: classify_room(&record),
                            source: record.source_file.clone(),
                            metadata: serde_json::json!({
                                "timestamp": record.timestamp,
                                "actor": record.actor,
                                "is_user": record.is_user,
                                "platform": record.platform,
                                "source_type": record.source_type,
                                "thread_id": record.thread_id,
                            }),
                            embedding,
                        };
                        let json = serde_json::to_string(&drawer).unwrap();
                        writeln!(writer, "{}", json).unwrap();
                        embedded += 1;
                    }
                }
                Err(e) => {
                    eprintln!("Embedding batch failed: {} — retrying in 2s", e);
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    // Retry once
                    match embed_batch(&client, &batch_texts) {
                        Ok(embeddings) => {
                            for (record, embedding) in batch_records.drain(..).zip(embeddings.into_iter()) {
                                let drawer = MemPalaceDrawer {
                                    id: record.content_hash.clone(),
                                    content: record.content.clone(),
                                    wing: platform_to_wing(&record.platform).to_string(),
                                    room: classify_room(&record),
                                    source: record.source_file.clone(),
                                    metadata: serde_json::json!({
                                        "timestamp": record.timestamp,
                                        "actor": record.actor,
                                        "is_user": record.is_user,
                                        "platform": record.platform,
                                        "source_type": record.source_type,
                                        "thread_id": record.thread_id,
                                    }),
                                    embedding,
                                };
                                let json = serde_json::to_string(&drawer).unwrap();
                                writeln!(writer, "{}", json).unwrap();
                                embedded += 1;
                            }
                        }
                        Err(e2) => {
                            eprintln!("Retry failed: {} — skipping batch", e2);
                            skipped += batch_records.len();
                            batch_records.clear();
                        }
                    }
                }
            }
            batch_texts.clear();

            // Flush periodically
            if embedded % 500 == 0 {
                writer.flush().unwrap();
            }

            // Progress report every 30 seconds
            if last_report.elapsed().as_secs() >= 30 {
                let elapsed = start.elapsed().as_secs_f64();
                let rate = embedded as f64 / elapsed;
                let remaining = (total - already_done - embedded - skipped) as f64 / rate;
                println!(
                    "  [{:.0}s] {}/{} embedded ({} skipped) — {:.0}/sec — ~{:.0}min remaining",
                    elapsed, embedded, total - already_done, skipped, rate, remaining / 60.0
                );
                last_report = Instant::now();
            }
        }
    }

    // Process remaining batch
    if !batch_texts.is_empty() {
        if let Ok(embeddings) = embed_batch(&client, &batch_texts) {
            for (record, embedding) in batch_records.drain(..).zip(embeddings.into_iter()) {
                let drawer = MemPalaceDrawer {
                    id: record.content_hash.clone(),
                    content: record.content.clone(),
                    wing: platform_to_wing(&record.platform).to_string(),
                    room: classify_room(&record),
                    source: record.source_file.clone(),
                    metadata: serde_json::json!({
                        "timestamp": record.timestamp,
                        "actor": record.actor,
                        "is_user": record.is_user,
                        "platform": record.platform,
                        "source_type": record.source_type,
                        "thread_id": record.thread_id,
                    }),
                    embedding,
                };
                let json = serde_json::to_string(&drawer).unwrap();
                writeln!(writer, "{}", json).unwrap();
                embedded += 1;
            }
        }
    }

    writer.flush().unwrap();

    let elapsed = start.elapsed();
    println!("\n=== Done ===");
    println!("  Total records: {}", total);
    println!("  Embedded:      {}", embedded);
    println!("  Skipped:       {}", skipped);
    println!("  Time:          {:.1?}", elapsed);
    println!("  Rate:          {:.0}/sec", embedded as f64 / elapsed.as_secs_f64());
    println!("  Output:        {}", output_path);
}
