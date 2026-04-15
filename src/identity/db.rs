//! SQLite persistence for the identity graph.
//!
//! Single portable file: identity.db
//! Tables: persons, identifiers, groups, connections, relationship_details, corrections

use rusqlite::{Connection, params};
use crate::identity::*;

/// Create or open an identity database.
pub fn open(path: &str) -> Result<Connection, String> {
    let conn = Connection::open(path)
        .map_err(|e| format!("Failed to open identity.db: {}", e))?;
    create_tables(&conn)?;
    Ok(conn)
}

fn create_tables(conn: &Connection) -> Result<(), String> {
    // WAL mode for much better write performance
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
        .map_err(|e| format!("Failed to set WAL mode: {}", e))?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS persons (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            canonical_name TEXT NOT NULL,
            first_name TEXT NOT NULL DEFAULT '',
            last_name TEXT NOT NULL DEFAULT '',
            gender TEXT NOT NULL DEFAULT '?',
            relationship_type TEXT NOT NULL DEFAULT 'unknown',
            first_seen TEXT,
            last_seen TEXT,
            interaction_count INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS identifiers (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            person_id INTEGER NOT NULL,
            platform TEXT NOT NULL,
            value TEXT NOT NULL,
            id_type TEXT NOT NULL,
            FOREIGN KEY (person_id) REFERENCES persons(id),
            UNIQUE(platform, value, id_type)
        );

        CREATE TABLE IF NOT EXISTS groups (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            person_id INTEGER NOT NULL,
            group_name TEXT NOT NULL,
            FOREIGN KEY (person_id) REFERENCES persons(id),
            UNIQUE(person_id, group_name)
        );

        CREATE TABLE IF NOT EXISTS connections (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            person_a_id INTEGER NOT NULL,
            person_b_id INTEGER NOT NULL,
            connection_type TEXT NOT NULL,
            note TEXT NOT NULL DEFAULT '',
            FOREIGN KEY (person_a_id) REFERENCES persons(id),
            FOREIGN KEY (person_b_id) REFERENCES persons(id)
        );

        CREATE TABLE IF NOT EXISTS relationship_details (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            person_id INTEGER NOT NULL,
            key TEXT NOT NULL,
            value TEXT NOT NULL,
            FOREIGN KEY (person_id) REFERENCES persons(id),
            UNIQUE(person_id, key)
        );

        CREATE TABLE IF NOT EXISTS corrections (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            person_id INTEGER NOT NULL,
            field TEXT NOT NULL,
            old_value TEXT NOT NULL DEFAULT '',
            new_value TEXT NOT NULL,
            source TEXT NOT NULL DEFAULT 'user',
            timestamp TEXT NOT NULL DEFAULT (datetime('now')),
            FOREIGN KEY (person_id) REFERENCES persons(id)
        );

        CREATE INDEX IF NOT EXISTS idx_identifiers_platform_value ON identifiers(platform, value);
        CREATE INDEX IF NOT EXISTS idx_identifiers_person ON identifiers(person_id);
        CREATE INDEX IF NOT EXISTS idx_persons_name ON persons(canonical_name);
        CREATE INDEX IF NOT EXISTS idx_persons_last_name ON persons(last_name);
        CREATE INDEX IF NOT EXISTS idx_groups_person ON groups(person_id);
        CREATE INDEX IF NOT EXISTS idx_corrections_person ON corrections(person_id);
        "
    ).map_err(|e| format!("Failed to create tables: {}", e))?;
    Ok(())
}

/// Insert a new person. Returns the new person ID.
pub fn insert_person(conn: &Connection, name: &str, gender: &str) -> Result<i64, String> {
    let (first, last) = parse_name(name);
    conn.execute(
        "INSERT INTO persons (canonical_name, first_name, last_name, gender) VALUES (?1, ?2, ?3, ?4)",
        params![name, first, last, gender],
    ).map_err(|e| format!("Failed to insert person: {}", e))?;
    Ok(conn.last_insert_rowid())
}

/// Find a person by exact identifier (platform + value).
pub fn find_by_identifier(conn: &Connection, platform: &str, value: &str) -> Option<i64> {
    conn.query_row(
        "SELECT person_id FROM identifiers WHERE platform = ?1 AND value = ?2",
        params![platform, value],
        |row| row.get(0),
    ).ok()
}

/// Find a person by exact full name (first + last).
pub fn find_by_full_name(conn: &Connection, name: &str) -> Option<i64> {
    let (first, last) = parse_name(name);
    if last.is_empty() {
        return None; // Never match on first name alone
    }
    conn.query_row(
        "SELECT id FROM persons WHERE first_name = ?1 AND last_name = ?2 COLLATE NOCASE",
        params![first, last],
        |row| row.get(0),
    ).ok()
}

/// Add an identifier to a person.
pub fn add_identifier(conn: &Connection, person_id: i64, platform: &str, value: &str, id_type: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO identifiers (person_id, platform, value, id_type) VALUES (?1, ?2, ?3, ?4)",
        params![person_id, platform, value, id_type],
    ).map_err(|e| format!("Failed to add identifier: {}", e))?;
    Ok(())
}

/// Add a person to a group.
pub fn add_to_group(conn: &Connection, person_id: i64, group: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO groups (person_id, group_name) VALUES (?1, ?2)",
        params![person_id, group],
    ).map_err(|e| format!("Failed to add to group: {}", e))?;
    Ok(())
}

/// Update interaction stats for a person.
pub fn update_interaction(conn: &Connection, person_id: i64, count: i64, first: Option<&str>, last: Option<&str>) -> Result<(), String> {
    conn.execute(
        "UPDATE persons SET interaction_count = interaction_count + ?2,
         first_seen = CASE WHEN first_seen IS NULL OR ?3 < first_seen THEN ?3 ELSE first_seen END,
         last_seen = CASE WHEN last_seen IS NULL OR ?4 > last_seen THEN ?4 ELSE last_seen END,
         updated_at = datetime('now')
         WHERE id = ?1",
        params![person_id, count, first, last],
    ).map_err(|e| format!("Failed to update interaction: {}", e))?;
    Ok(())
}

/// Add a user correction.
pub fn add_correction(conn: &Connection, person_id: i64, field: &str, old_value: &str, new_value: &str, source: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO corrections (person_id, field, old_value, new_value, source) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![person_id, field, old_value, new_value, source],
    ).map_err(|e| format!("Failed to add correction: {}", e))?;

    // Apply the correction immediately
    match field {
        "canonical_name" => {
            let (first, last) = parse_name(new_value);
            conn.execute(
                "UPDATE persons SET canonical_name = ?2, first_name = ?3, last_name = ?4, updated_at = datetime('now') WHERE id = ?1",
                params![person_id, new_value, first, last],
            ).map_err(|e| format!("Failed to apply correction: {}", e))?;
        }
        "gender" => {
            conn.execute("UPDATE persons SET gender = ?2, updated_at = datetime('now') WHERE id = ?1", params![person_id, new_value])
                .map_err(|e| format!("Failed to apply correction: {}", e))?;
        }
        "relationship_type" => {
            conn.execute("UPDATE persons SET relationship_type = ?2, updated_at = datetime('now') WHERE id = ?1", params![person_id, new_value])
                .map_err(|e| format!("Failed to apply correction: {}", e))?;
        }
        "group_add" => {
            add_to_group(conn, person_id, new_value)?;
        }
        "group_remove" => {
            conn.execute("DELETE FROM groups WHERE person_id = ?1 AND group_name = ?2", params![person_id, new_value])
                .map_err(|e| format!("Failed to apply correction: {}", e))?;
        }
        _ => {} // Unknown field — store correction but don't auto-apply
    }
    Ok(())
}

/// Apply all stored corrections (used after re-processing).
pub fn apply_corrections(conn: &Connection) -> Result<usize, String> {
    let mut stmt = conn.prepare(
        "SELECT person_id, field, new_value FROM corrections ORDER BY timestamp ASC"
    ).map_err(|e| format!("Failed to query corrections: {}", e))?;

    let corrections: Vec<(i64, String, String)> = stmt.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    }).map_err(|e| format!("Failed to read corrections: {}", e))?
    .filter_map(|r| r.ok())
    .collect();

    let count = corrections.len();
    for (person_id, field, new_value) in &corrections {
        match field.as_str() {
            "canonical_name" => {
                let (first, last) = parse_name(new_value);
                let _ = conn.execute(
                    "UPDATE persons SET canonical_name = ?2, first_name = ?3, last_name = ?4 WHERE id = ?1",
                    params![person_id, new_value, first, last],
                );
            }
            "gender" => {
                let _ = conn.execute("UPDATE persons SET gender = ?2 WHERE id = ?1", params![person_id, new_value]);
            }
            "relationship_type" => {
                let _ = conn.execute("UPDATE persons SET relationship_type = ?2 WHERE id = ?1", params![person_id, new_value]);
            }
            _ => {}
        }
    }
    Ok(count)
}

/// Export all persons as a JSON-serializable structure.
pub fn export_all(conn: &Connection) -> Result<Vec<Person>, String> {
    let mut stmt = conn.prepare(
        "SELECT id, canonical_name, first_name, last_name, gender, first_seen, last_seen, interaction_count
         FROM persons ORDER BY interaction_count DESC"
    ).map_err(|e| format!("Query failed: {}", e))?;

    let persons: Vec<Person> = stmt.query_map([], |row| {
        let id: i64 = row.get(0)?;
        Ok(Person {
            id,
            canonical_name: row.get(1)?,
            first_name: row.get(2)?,
            last_name: row.get(3)?,
            gender: row.get(4)?,
            identifiers: Vec::new(), // Filled below
            groups: Vec::new(),
            connections: Vec::new(),
            first_seen: row.get(5)?,
            last_seen: row.get(6)?,
            interaction_count: row.get(7)?,
            platforms: Vec::new(),
        })
    }).map_err(|e| format!("Failed to read persons: {}", e))?
    .filter_map(|r| r.ok())
    .collect();

    // Fill identifiers, groups, platforms
    let mut result = Vec::new();
    for mut person in persons {
        // Identifiers
        let mut id_stmt = conn.prepare("SELECT platform, value, id_type FROM identifiers WHERE person_id = ?1")
            .map_err(|e| format!("Query failed: {}", e))?;
        person.identifiers = id_stmt.query_map(params![person.id], |row| {
            Ok(Identifier {
                platform: row.get(0)?,
                value: row.get(1)?,
                id_type: IdentifierType::from_str(&row.get::<_, String>(2)?),
            })
        }).map_err(|e| format!("Failed: {}", e))?.filter_map(|r| r.ok()).collect();

        // Platforms (deduplicated from identifiers)
        let mut platforms: Vec<String> = person.identifiers.iter().map(|i| i.platform.clone()).collect();
        platforms.sort();
        platforms.dedup();
        person.platforms = platforms;

        // Groups
        let mut grp_stmt = conn.prepare("SELECT group_name FROM groups WHERE person_id = ?1")
            .map_err(|e| format!("Query failed: {}", e))?;
        person.groups = grp_stmt.query_map(params![person.id], |row| row.get(0))
            .map_err(|e| format!("Failed: {}", e))?.filter_map(|r| r.ok()).collect();

        result.push(person);
    }

    Ok(result)
}
