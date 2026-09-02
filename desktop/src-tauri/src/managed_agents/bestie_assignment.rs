//! Durable, owner-and-relay-scoped Bestie designation storage.

use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

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

#[derive(Clone)]
struct ScopedAssignment {
    agent_pubkey: String,
    path: PathBuf,
}

fn retention_db_paths(base_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let retention_dir = base_dir.join("retention");
    let entries = match fs::read_dir(&retention_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "failed to read retention directory {}: {error}",
                retention_dir.display()
            ))
        }
    };

    let mut paths = Vec::new();
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
        paths.push(path);
    }
    paths.sort();
    Ok(paths)
}

fn matching_assignments(
    base_dir: &Path,
    agent_pubkey: &str,
) -> Result<Vec<ScopedAssignment>, String> {
    let normalized = agent_pubkey.trim().to_ascii_lowercase();
    let mut assignments = Vec::new();
    // Read and validate every scope before mutating any of them. A broken later
    // database therefore cannot leave an already-cleared prefix behind.
    for path in retention_db_paths(base_dir)? {
        let conn = open_retention_db(&path)?;
        ensure_table(&conn)?;
        if assignment_matches(&conn, &normalized)? {
            assignments.push(ScopedAssignment {
                agent_pubkey: normalized.clone(),
                path,
            });
        }
    }
    Ok(assignments)
}

fn clear_scope(assignment: &ScopedAssignment) -> Result<(), String> {
    let conn = open_retention_db(&assignment.path)?;
    conn.execute(
        "DELETE FROM bestie_assignments WHERE singleton = 1 AND agent_pubkey = ?1",
        params![assignment.agent_pubkey],
    )
    .map_err(|error| {
        format!(
            "failed to clear bestie assignment in {}: {error}",
            assignment.path.display()
        )
    })?;
    Ok(())
}

fn restore_assignments(assignments: &[ScopedAssignment]) -> Result<(), String> {
    let mut failures = Vec::new();
    for assignment in assignments {
        let result = open_retention_db(&assignment.path).and_then(|mut conn| {
            replace_assignment(&mut conn, &assignment.agent_pubkey).map(|_| ())
        });
        if let Err(error) = result {
            failures.push(format!("{}: {error}", assignment.path.display()));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "failed to restore bestie assignments: {}",
            failures.join("; ")
        ))
    }
}

fn clear_scoped_assignments(
    assignments: &[ScopedAssignment],
    mut clear: impl FnMut(&ScopedAssignment) -> Result<(), String>,
) -> Result<(), String> {
    let mut cleared = Vec::new();
    for assignment in assignments {
        if let Err(error) = clear(assignment) {
            return match restore_assignments(&cleared) {
                Ok(()) => Err(error),
                Err(restore_error) => Err(format!("{error}; {restore_error}")),
            };
        }
        cleared.push(assignment.clone());
    }
    Ok(())
}

/// Run agent deletion work with this agent's community-scoped Bestie
/// assignments temporarily cleared.
///
/// Call this while holding `managed_agents_store_lock`. Every matching scope is
/// snapshotted before the first write. A partial clear, or any later stop/save
/// failure returned by `delete`, restores the snapshot before the error is
/// propagated. Assignments remain cleared only when `delete` succeeds.
pub fn with_agent_assignments_cleared<T>(
    base_dir: &Path,
    agent_pubkey: &str,
    delete: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let assignments = matching_assignments(base_dir, agent_pubkey)?;
    clear_scoped_assignments(&assignments, clear_scope)?;
    match delete() {
        Ok(value) => Ok(value),
        Err(error) => match restore_assignments(&assignments) {
            Ok(()) => Err(error),
            Err(restore_error) => Err(format!("{error}; {restore_error}")),
        },
    }
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

        with_agent_assignments_cleared(dir.path(), &agent, || Ok(()))
            .unwrap_or_else(|error| panic!("clear agent assignments: {error}"));
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

    #[test]
    fn later_scope_clear_failure_restores_the_already_cleared_prefix() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("temp dir: {error}"));
        let retention_dir = dir.path().join("retention");
        fs::create_dir_all(&retention_dir)
            .unwrap_or_else(|error| panic!("create retention dir: {error}"));
        let agent = "a".repeat(64);
        for name in ["first.db", "second.db"] {
            replace_assignment(
                &mut open_retention_db(&retention_dir.join(name))
                    .unwrap_or_else(|error| panic!("open {name}: {error}")),
                &agent,
            )
            .unwrap_or_else(|error| panic!("assign {name}: {error}"));
        }

        let assignments = matching_assignments(dir.path(), &agent)
            .unwrap_or_else(|error| panic!("snapshot assignments: {error}"));
        let result = clear_scoped_assignments(&assignments, |assignment| {
            if assignment.path.ends_with("second.db") {
                Err("injected later retention DB failure".to_string())
            } else {
                clear_scope(assignment)
            }
        });

        assert!(result.is_err());
        for name in ["first.db", "second.db"] {
            let conn = open_retention_db(&retention_dir.join(name))
                .unwrap_or_else(|error| panic!("reopen {name}: {error}"));
            assert!(assignment_matches(&conn, &agent)
                .unwrap_or_else(|error| panic!("read {name}: {error}")));
        }
    }

    fn assert_later_deletion_failure_restores_assignment(failure: &str) {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("temp dir: {error}"));
        let retention_dir = dir.path().join("retention");
        fs::create_dir_all(&retention_dir)
            .unwrap_or_else(|error| panic!("create retention dir: {error}"));
        let path = retention_dir.join("owner.db");
        let agent = "a".repeat(64);
        replace_assignment(
            &mut open_retention_db(&path)
                .unwrap_or_else(|error| panic!("open assignment db: {error}")),
            &agent,
        )
        .unwrap_or_else(|error| panic!("assign agent: {error}"));

        let result = with_agent_assignments_cleared(dir.path(), &agent, || {
            Err::<(), _>(failure.to_string())
        });

        assert_eq!(result, Err(failure.to_string()));
        let conn = open_retention_db(&path)
            .unwrap_or_else(|error| panic!("reopen assignment db: {error}"));
        assert!(assignment_matches(&conn, &agent)
            .unwrap_or_else(|error| panic!("read restored assignment: {error}")));
    }

    #[test]
    fn stop_failure_after_cleanup_restores_assignment() {
        assert_later_deletion_failure_restores_assignment("injected stop failure");
    }

    #[test]
    fn save_failure_after_cleanup_restores_assignment() {
        assert_later_deletion_failure_restores_assignment("injected save failure");
    }
}
