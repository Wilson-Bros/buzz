//! Durable, owner-and-relay-scoped Bestie designation storage.

use std::{fs, io::ErrorKind, path::Path};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::retention::open_retention_db;

/// The one durable Bestie designation in a retention scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BestieAssignment {
    pub agent_pubkey: String,
}

fn ensure_table(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS bestie_assignments (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
            agent_pubkey TEXT NOT NULL
        );",
    )
    .map_err(|error| format!("failed to create bestie assignment table: {error}"))
}

/// Read the designation for the already-scoped retention database.
pub fn get_assignment(conn: &Connection) -> Result<Option<BestieAssignment>, String> {
    ensure_table(conn)?;
    conn.query_row(
        "SELECT agent_pubkey FROM bestie_assignments WHERE singleton = 1",
        [],
        |row| {
            Ok(BestieAssignment {
                agent_pubkey: row.get(0)?,
            })
        },
    )
    .optional()
    .map_err(|error| format!("failed to read bestie assignment: {error}"))
}

/// Atomically create or replace the one designation in this scope.
pub fn replace_assignment(
    conn: &mut Connection,
    agent_pubkey: &str,
) -> Result<BestieAssignment, String> {
    ensure_table(conn)?;
    let normalized = agent_pubkey.trim().to_ascii_lowercase();
    let transaction = conn
        .transaction()
        .map_err(|error| format!("failed to begin bestie assignment transaction: {error}"))?;
    transaction
        .execute(
            "INSERT INTO bestie_assignments (singleton, agent_pubkey)
             VALUES (1, ?1)
             ON CONFLICT(singleton) DO UPDATE SET agent_pubkey = excluded.agent_pubkey",
            params![normalized],
        )
        .map_err(|error| format!("failed to replace bestie assignment: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("failed to commit bestie assignment: {error}"))?;
    get_assignment(conn)?.ok_or_else(|| "bestie assignment was not persisted".to_string())
}

/// Clear the designation without changing or stopping the agent.
pub fn clear_assignment(conn: &mut Connection) -> Result<(), String> {
    ensure_table(conn)?;
    let transaction = conn
        .transaction()
        .map_err(|error| format!("failed to begin bestie clear transaction: {error}"))?;
    transaction
        .execute("DELETE FROM bestie_assignments WHERE singleton = 1", [])
        .map_err(|error| format!("failed to clear bestie assignment: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("failed to commit bestie clear: {error}"))
}

/// Whether the same agent is still designated after an asynchronous operation.
pub fn assignment_matches(conn: &Connection, agent_pubkey: &str) -> Result<bool, String> {
    ensure_table(conn)?;
    let normalized = agent_pubkey.trim().to_ascii_lowercase();
    Ok(get_assignment(conn)?.is_some_and(|assignment| assignment.agent_pubkey == normalized))
}

/// Clear this agent's designation from every existing community scope.
///
/// Call this while holding `managed_agents_store_lock` and before removing the
/// agent record. If one database cannot be updated, deletion stops while the
/// agent still exists, so no scope can be left pointing at a deleted agent.
pub fn clear_agent_assignments(base_dir: &Path, agent_pubkey: &str) -> Result<usize, String> {
    let retention_dir = base_dir.join("retention");
    let entries = match fs::read_dir(&retention_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(format!(
                "failed to read retention directory {}: {error}",
                retention_dir.display()
            ))
        }
    };

    let normalized = agent_pubkey.trim().to_ascii_lowercase();
    let mut cleared = 0;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "failed to inspect retention directory {}: {error}",
                retention_dir.display()
            )
        })?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("db") {
            continue;
        }
        let conn = open_retention_db(&path)?;
        ensure_table(&conn)?;
        cleared += conn
            .execute(
                "DELETE FROM bestie_assignments WHERE singleton = 1 AND agent_pubkey = ?1",
                params![normalized],
            )
            .map_err(|error| {
                format!(
                    "failed to clear bestie assignment in {}: {error}",
                    path.display()
                )
            })?;
    }
    Ok(cleared)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection() -> Connection {
        Connection::open_in_memory().unwrap_or_else(|error| panic!("open test db: {error}"))
    }

    #[test]
    fn assignment_is_singleton_and_idempotent() {
        let mut conn = connection();
        let first = replace_assignment(&mut conn, &"A".repeat(64))
            .unwrap_or_else(|error| panic!("assign first: {error}"));
        assert_eq!(first.agent_pubkey, "a".repeat(64));

        let same = replace_assignment(&mut conn, &"a".repeat(64))
            .unwrap_or_else(|error| panic!("reassign same: {error}"));
        assert_eq!(same.agent_pubkey, "a".repeat(64));

        let replaced = replace_assignment(&mut conn, &"b".repeat(64))
            .unwrap_or_else(|error| panic!("replace: {error}"));
        assert_eq!(replaced.agent_pubkey, "b".repeat(64));
    }

    #[test]
    fn stale_resolver_is_fenced_after_replace_and_clear_is_idempotent() {
        let mut conn = connection();
        replace_assignment(&mut conn, &"a".repeat(64))
            .unwrap_or_else(|error| panic!("assign: {error}"));
        replace_assignment(&mut conn, &"b".repeat(64))
            .unwrap_or_else(|error| panic!("replace: {error}"));
        assert!(!assignment_matches(&conn, &"a".repeat(64))
            .unwrap_or_else(|error| panic!("check stale assignment: {error}")));
        clear_assignment(&mut conn).unwrap_or_else(|error| panic!("clear: {error}"));
        clear_assignment(&mut conn).unwrap_or_else(|error| panic!("clear again: {error}"));
        assert_eq!(
            get_assignment(&conn).unwrap_or_else(|error| panic!("read: {error}")),
            None
        );
    }

    #[test]
    fn deleting_agent_clears_every_matching_scope_and_preserves_other_assignments() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("temp dir: {error}"));
        let retention_dir = dir.path().join("retention");
        fs::create_dir_all(&retention_dir)
            .unwrap_or_else(|error| panic!("create retention dir: {error}"));
        let agent = "a".repeat(64);
        let other = "b".repeat(64);
        let first_path = retention_dir.join("first.db");
        let second_path = retention_dir.join("second.db");
        let third_path = retention_dir.join("third.db");
        replace_assignment(
            &mut open_retention_db(&first_path)
                .unwrap_or_else(|error| panic!("open first db: {error}")),
            &agent,
        )
        .unwrap_or_else(|error| panic!("assign first scope: {error}"));
        replace_assignment(
            &mut open_retention_db(&second_path)
                .unwrap_or_else(|error| panic!("open second db: {error}")),
            &agent,
        )
        .unwrap_or_else(|error| panic!("assign second scope: {error}"));
        replace_assignment(
            &mut open_retention_db(&third_path)
                .unwrap_or_else(|error| panic!("open third db: {error}")),
            &other,
        )
        .unwrap_or_else(|error| panic!("assign third scope: {error}"));

        assert_eq!(
            clear_agent_assignments(dir.path(), &agent)
                .unwrap_or_else(|error| panic!("clear agent assignments: {error}")),
            2
        );
        assert_eq!(
            get_assignment(
                &open_retention_db(&first_path)
                    .unwrap_or_else(|error| panic!("reopen first db: {error}"))
            )
            .unwrap_or_else(|error| panic!("read first scope: {error}")),
            None
        );
        assert_eq!(
            get_assignment(
                &open_retention_db(&third_path)
                    .unwrap_or_else(|error| panic!("reopen third db: {error}"))
            )
            .unwrap_or_else(|error| panic!("read third scope: {error}"))
            .map(|assignment| assignment.agent_pubkey),
            Some(other)
        );
    }
}
