use rusqlite::Connection;

use super::errors::Result;

pub const SCHEMA_VERSION: i64 = 3;

pub fn migrate(connection: &mut Connection) -> Result<()> {
    connection.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
    )?;
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);",
    )?;

    apply(
        connection,
        1,
        r#"
        CREATE TABLE IF NOT EXISTS agents (
          id TEXT PRIMARY KEY, name TEXT NOT NULL, provider TEXT NOT NULL, model TEXT NOT NULL,
          reasoning_effort TEXT NOT NULL, status TEXT NOT NULL, parent_agent_id TEXT,
          current_task_id TEXT, config_json TEXT NOT NULL, updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS tasks (
          id TEXT PRIMARY KEY, status TEXT NOT NULL, created_by TEXT NOT NULL, objective TEXT NOT NULL,
          assigned_to_json TEXT NOT NULL, source_file TEXT, task_json TEXT NOT NULL,
          result_json TEXT, updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS events (
          id TEXT PRIMARY KEY, type TEXT NOT NULL, timestamp TEXT NOT NULL, source TEXT NOT NULL,
          target TEXT, task_id TEXT, payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS resources (
          agent_id TEXT PRIMARY KEY, status TEXT NOT NULL, reset_at TEXT,
          details_json TEXT NOT NULL, updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS sessions (
          agent_id TEXT PRIMARY KEY, provider TEXT NOT NULL, provider_session_id TEXT NOT NULL,
          metadata_json TEXT NOT NULL, updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS scheduler_jobs (
          id TEXT PRIMARY KEY, kind TEXT NOT NULL, run_at TEXT NOT NULL,
          payload_json TEXT NOT NULL, status TEXT NOT NULL, updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS authority_messages (
          id TEXT PRIMARY KEY, source TEXT NOT NULL, target TEXT NOT NULL, authority INTEGER NOT NULL,
          scope TEXT NOT NULL, content TEXT NOT NULL, status TEXT NOT NULL, timestamp TEXT NOT NULL,
          metadata_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS decisions (
          id TEXT PRIMARY KEY, topic TEXT NOT NULL, question TEXT NOT NULL, recommendation_json TEXT,
          alternatives_json TEXT NOT NULL, impact TEXT NOT NULL, requires TEXT NOT NULL,
          effective_json TEXT, decided_by TEXT, status TEXT NOT NULL, updated_at TEXT NOT NULL
        );
    "#,
    )?;

    apply(
        connection,
        2,
        r#"
        CREATE TABLE IF NOT EXISTS task_runs (
          task_id TEXT NOT NULL, agent_id TEXT NOT NULL, attempt INTEGER NOT NULL DEFAULT 1,
          status TEXT NOT NULL, result_json TEXT, provider TEXT, provider_session_id TEXT,
          started_at TEXT, completed_at TEXT, updated_at TEXT NOT NULL,
          PRIMARY KEY(task_id, agent_id),
          FOREIGN KEY(task_id) REFERENCES tasks(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_events_task ON events(task_id, timestamp);
        CREATE INDEX IF NOT EXISTS idx_scheduler_due ON scheduler_jobs(status, run_at);
    "#,
    )?;

    if !column_exists(connection, "tasks", "content_hash")? {
        connection.execute("ALTER TABLE tasks ADD COLUMN content_hash TEXT", [])?;
    }
    if !column_exists(connection, "tasks", "revision")? {
        connection.execute(
            "ALTER TABLE tasks ADD COLUMN revision INTEGER NOT NULL DEFAULT 1",
            [],
        )?;
    }
    if !column_exists(connection, "sessions", "fingerprint")? {
        connection.execute(
            "ALTER TABLE sessions ADD COLUMN fingerprint TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    if !column_exists(connection, "scheduler_jobs", "agent_id")? {
        connection.execute("ALTER TABLE scheduler_jobs ADD COLUMN agent_id TEXT", [])?;
    }
    if !column_exists(connection, "scheduler_jobs", "attempts")? {
        connection.execute(
            "ALTER TABLE scheduler_jobs ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    apply(connection, 3, "CREATE UNIQUE INDEX IF NOT EXISTS idx_scheduler_agent_pending ON scheduler_jobs(kind, agent_id) WHERE status IN ('PENDING','RUNNING') AND agent_id IS NOT NULL;")?;
    Ok(())
}

fn apply(connection: &mut Connection, version: i64, sql: &str) -> Result<()> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=?)",
        [version],
        |row| row.get(0),
    )?;
    if exists {
        return Ok(());
    }
    let transaction = connection.transaction()?;
    transaction.execute_batch(sql)?;
    transaction.execute(
        "INSERT INTO schema_migrations(version, applied_at) VALUES(?, ?)",
        rusqlite::params![version, chrono::Utc::now().to_rfc3339()],
    )?;
    transaction.commit()?;
    Ok(())
}

fn column_exists(connection: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = statement.query_map([], |row| row.get::<_, String>(1))?;
    for name in names {
        if name? == column {
            return Ok(true);
        }
    }
    Ok(false)
}
