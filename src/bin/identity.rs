//! Identity Graph CLI — resolve, export, correct.
//!
//! Usage:
//!   identity resolve --records export.jsonl --contacts ./AddressBook/ --snapchat-friends friends.zip --output identity.db
//!   identity export --db identity.db --format json > people.json
//!   identity correct --db identity.db --person "Jenny Lieu" --set group=romantic

use neuron::identity::{db, resolver::IdentityResolver};
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter("neuron=info")
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: identity <resolve|export|correct> [options]");
        eprintln!("");
        eprintln!("Commands:");
        eprintln!("  resolve  --records <jsonl> [--contacts <dir>] [--snapchat-friends <path>] --output <db>");
        eprintln!("  export   --db <db> [--format json|csv]");
        eprintln!("  correct  --db <db> --person <name> --set <field>=<value>");
        std::process::exit(1);
    }

    match args[1].as_str() {
        "resolve" => cmd_resolve(&args[2..]),
        "export" => cmd_export(&args[2..]),
        "correct" => cmd_correct(&args[2..]),
        _ => {
            eprintln!("Unknown command: {}", args[1]);
            std::process::exit(1);
        }
    }
}

fn cmd_resolve(args: &[String]) {
    let mut records_path = String::new();
    let mut contacts_dir = String::new();
    let mut snapchat_friends = String::new();
    let mut output_db = String::from("identity.db");

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--records" => { i += 1; records_path = args[i].clone(); }
            "--contacts" => { i += 1; contacts_dir = args[i].clone(); }
            "--snapchat-friends" => { i += 1; snapchat_friends = args[i].clone(); }
            "--output" => { i += 1; output_db = args[i].clone(); }
            _ => { eprintln!("Unknown option: {}", args[i]); }
        }
        i += 1;
    }

    if records_path.is_empty() {
        eprintln!("--records is required");
        std::process::exit(1);
    }

    println!("=== Identity Graph Resolver ===");
    println!("Records: {}", records_path);
    println!("Output:  {}", output_db);

    let start = Instant::now();

    // Open/create database
    let conn = db::open(&output_db).expect("Failed to open identity.db");

    // Build resolver with all available mappings
    let mut resolver = IdentityResolver::new();

    if !contacts_dir.is_empty() {
        println!("Loading contacts from: {}", contacts_dir);
        resolver.load_contacts(&PathBuf::from(&contacts_dir));
    }

    if !snapchat_friends.is_empty() {
        println!("Loading Snapchat friends from: {}", snapchat_friends);
        resolver.load_snapchat_friends(&PathBuf::from(&snapchat_friends));
    }

    // Load Instagram actors from the same JSONL
    println!("Loading Instagram actors...");
    resolver.load_instagram_actors(&PathBuf::from(&records_path));

    // Resolve
    println!("Resolving identities...");
    let stats = resolver.resolve_all(&PathBuf::from(&records_path), &conn)
        .expect("Resolution failed");

    let elapsed = start.elapsed();
    println!("");
    println!("=== Done ({:.1?}) ===", elapsed);
    println!("  Records processed:  {}", stats.records_processed);
    println!("  Persons created:    {}", stats.persons_created);
    println!("  Identifiers linked: {}", stats.identifiers_linked);
    println!("  Corrections applied:{}", stats.corrections_applied);
    println!("  Errors:             {}", stats.errors);
    println!("  Output:             {}", output_db);
}

fn cmd_export(args: &[String]) {
    let mut db_path = String::from("identity.db");
    let mut format = String::from("json");

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--db" => { i += 1; db_path = args[i].clone(); }
            "--format" => { i += 1; format = args[i].clone(); }
            _ => {}
        }
        i += 1;
    }

    let conn = db::open(&db_path).expect("Failed to open identity.db");
    let persons = db::export_all(&conn).expect("Export failed");

    match format.as_str() {
        "json" => {
            // Export as the format EVA App expects
            let output: Vec<serde_json::Value> = persons.iter().map(|p| {
                serde_json::json!({
                    "name": p.canonical_name,
                    "interactions": p.interaction_count,
                    "platforms": p.platforms,
                    "first_seen": p.first_seen,
                    "last_seen": p.last_seen,
                    "gender": p.gender,
                    "groups": p.groups,
                    "identifiers": p.identifiers.iter().map(|id| {
                        serde_json::json!({
                            "platform": id.platform,
                            "value": id.value,
                            "type": id.id_type.to_string(),
                        })
                    }).collect::<Vec<_>>(),
                })
            }).collect();
            println!("{}", serde_json::to_string_pretty(&output).unwrap());
        }
        _ => {
            eprintln!("Unknown format: {}", format);
        }
    }
}

fn cmd_correct(args: &[String]) {
    let mut db_path = String::from("identity.db");
    let mut person_name = String::new();
    let mut set_field = String::new();
    let mut set_value = String::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--db" => { i += 1; db_path = args[i].clone(); }
            "--person" => { i += 1; person_name = args[i].clone(); }
            "--set" => {
                i += 1;
                let parts: Vec<&str> = args[i].splitn(2, '=').collect();
                if parts.len() == 2 {
                    set_field = parts[0].to_string();
                    set_value = parts[1].to_string();
                }
            }
            _ => {}
        }
        i += 1;
    }

    if person_name.is_empty() || set_field.is_empty() {
        eprintln!("Usage: identity correct --db <db> --person <name> --set <field>=<value>");
        std::process::exit(1);
    }

    let conn = db::open(&db_path).expect("Failed to open identity.db");
    let person_id = db::find_by_full_name(&conn, &person_name)
        .expect(&format!("Person not found: {}", person_name));

    db::add_correction(&conn, person_id, &set_field, "", &set_value, "user")
        .expect("Failed to add correction");

    println!("Correction applied: {} {} = {}", person_name, set_field, set_value);
}
