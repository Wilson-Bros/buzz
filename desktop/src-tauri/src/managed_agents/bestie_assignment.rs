//! Durable, owner-and-relay-scoped Bestie designation storage.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

/// The one durable Bestie designation in a retention scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BestieAssignment {
    pub agent_pubkey: String,
    pub canonical_channel_id: Option<String>,
}

fn ensure_table(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS bestie_assignments (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
            agent_pubkey TEXT NOT NULL,
            canonical_channel_id TEXT
        );",
    )
    .map_err(|error| format!("failed to create bestie assignment table: {error}"))
}

/// Read the designation for the already-scoped retention database.
pub fn get_assignment(conn: &Connection) -> Result<Option<BestieAssignment>, String> {
    ensure_table(conn)?;
    conn.query_row(
        "SELECT agent_pubkey, canonical_channel_id
         FROM bestie_assignments WHERE singleton = 1",
        [],
        |row| {
            Ok(BestieAssignment {
                agent_pubkey: row.get(0)?,
                canonical_channel_id: row.get(1)?,
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
            "INSERT INTO bestie_assignments (singleton, agent_pubkey, canonical_channel_id)
             VALUES (1, ?1, NULL)
             ON CONFLICT(singleton) DO UPDATE SET
                agent_pubkey = excluded.agent_pubkey,
                canonical_channel_id = CASE
                    WHEN bestie_assignments.agent_pubkey = excluded.agent_pubkey
                    THEN bestie_assignments.canonical_channel_id
                    ELSE NULL
                END",
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

/// Repair the navigation cache only if the same agent is still designated.
pub fn cache_channel_if_current(
    conn: &Connection,
    agent_pubkey: &str,
    channel_id: &str,
) -> Result<bool, String> {
    ensure_table(conn)?;
    let changed = conn
        .execute(
            "UPDATE bestie_assignments SET canonical_channel_id = ?1
             WHERE singleton = 1 AND agent_pubkey = ?2",
            params![channel_id, agent_pubkey.trim().to_ascii_lowercase()],
        )
        .map_err(|error| format!("failed to cache bestie channel: {error}"))?;
    Ok(changed == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection() -> Connection {
        Connection::open_in_memory().unwrap_or_else(|error| panic!("open test db: {error}"))
    }

    #[test]
    fn assignment_is_singleton_idempotent_and_replacement_clears_cache() {
        let mut conn = connection();
        let first = replace_assignment(&mut conn, &"A".repeat(64))
            .unwrap_or_else(|error| panic!("assign first: {error}"));
        assert_eq!(first.agent_pubkey, "a".repeat(64));
        assert!(
            cache_channel_if_current(&conn, &"a".repeat(64), "channel-a")
                .unwrap_or_else(|error| panic!("cache channel: {error}"))
        );

        let same = replace_assignment(&mut conn, &"a".repeat(64))
            .unwrap_or_else(|error| panic!("reassign same: {error}"));
        assert_eq!(same.canonical_channel_id.as_deref(), Some("channel-a"));

        let replaced = replace_assignment(&mut conn, &"b".repeat(64))
            .unwrap_or_else(|error| panic!("replace: {error}"));
        assert_eq!(replaced.agent_pubkey, "b".repeat(64));
        assert_eq!(replaced.canonical_channel_id, None);
    }

    #[test]
    fn stale_resolver_cannot_cache_after_replace_and_clear_is_idempotent() {
        let mut conn = connection();
        replace_assignment(&mut conn, &"a".repeat(64))
            .unwrap_or_else(|error| panic!("assign: {error}"));
        replace_assignment(&mut conn, &"b".repeat(64))
            .unwrap_or_else(|error| panic!("replace: {error}"));
        assert!(!cache_channel_if_current(&conn, &"a".repeat(64), "stale")
            .unwrap_or_else(|error| panic!("stale cache: {error}")));
        clear_assignment(&mut conn).unwrap_or_else(|error| panic!("clear: {error}"));
        clear_assignment(&mut conn).unwrap_or_else(|error| panic!("clear again: {error}"));
        assert_eq!(
            get_assignment(&conn).unwrap_or_else(|error| panic!("read: {error}")),
            None
        );
    }
}
