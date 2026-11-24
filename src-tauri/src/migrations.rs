//! Migrations: `MIGRATIONS` is the ordered list of all schema changes, and `user_version`
//! is "how many entries have been applied".
//!
//! Forward only, no down migrations. Rollback relies on the pre-migration file backup, not
//! on an inverse script that nobody ever runs.

use rusqlite::Connection;

use crate::message::{Message, MessageKey, Res};

/// Migration scripts executed in order. The script at index `i` moves the database from
/// version `i` to `i + 1`.
///
/// Just append new entries; never change a single character of an already released entry.
pub const MIGRATIONS: &[&str] = &[
    V1_INITIAL_SCHEMA,
    V2_TASK_CLASSIFICATION_CASCADE,
    V3_CLAMP_SUBTASK_PRIORITY,
    V4_TASK_SCHEDULE,
    V5_TASK_CREATED_AT,
];

/// v1: project / module / submodule / task—two classification levels plus one subtask level.
const V1_INITIAL_SCHEMA: &str = r#"
CREATE TABLE project (
  id          INTEGER PRIMARY KEY,
  name        TEXT    NOT NULL,
  sort_order  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE module (
  id          INTEGER PRIMARY KEY,
  project_id  INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
  name        TEXT    NOT NULL,
  sort_order  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE submodule (
  id          INTEGER PRIMARY KEY,
  module_id   INTEGER NOT NULL REFERENCES module(id) ON DELETE CASCADE,
  name        TEXT    NOT NULL,
  sort_order  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE task (
  id             INTEGER PRIMARY KEY,
  project_id     INTEGER REFERENCES project(id) ON DELETE CASCADE,  -- NULL = "My Todos"
  module_id      INTEGER REFERENCES module(id) ON DELETE SET NULL,
  submodule_id   INTEGER REFERENCES submodule(id) ON DELETE SET NULL,
  parent_task_id INTEGER REFERENCES task(id) ON DELETE CASCADE,     -- NULL = top-level task
  title          TEXT    NOT NULL,
  status         TEXT    NOT NULL DEFAULT 'todo' CHECK (status IN ('todo','doing','done')),
  priority       TEXT    NOT NULL DEFAULT 'mid'  CHECK (priority IN ('high','mid','low')),
  note           TEXT    NOT NULL DEFAULT '',
  sort_order     INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_task_project ON task(project_id);
CREATE INDEX idx_task_module  ON task(module_id);
CREATE INDEX idx_task_parent  ON task(parent_task_id);
"#;

/// v2: when a module / submodule is deleted, the tasks inside it are deleted too instead of
/// falling back to a classification.
///
/// SQLite cannot change a foreign key action in place, so the table must be rebuilt.
/// Foreign keys are already disabled during migration, so the whole table can be copied;
/// the self-reference in `parent_task_id` is updated along with the table name.
const V2_TASK_CLASSIFICATION_CASCADE: &str = r#"
CREATE TABLE task_v2 (
  id             INTEGER PRIMARY KEY,
  project_id     INTEGER REFERENCES project(id) ON DELETE CASCADE,  -- NULL = "My Todos"
  module_id      INTEGER REFERENCES module(id) ON DELETE CASCADE,
  submodule_id   INTEGER REFERENCES submodule(id) ON DELETE CASCADE,
  parent_task_id INTEGER REFERENCES task_v2(id) ON DELETE CASCADE,  -- NULL = top-level task
  title          TEXT    NOT NULL,
  status         TEXT    NOT NULL DEFAULT 'todo' CHECK (status IN ('todo','doing','done')),
  priority       TEXT    NOT NULL DEFAULT 'mid'  CHECK (priority IN ('high','mid','low')),
  note           TEXT    NOT NULL DEFAULT '',
  sort_order     INTEGER NOT NULL DEFAULT 0
);

INSERT INTO task_v2 (id, project_id, module_id, submodule_id, parent_task_id,
                     title, status, priority, note, sort_order)
SELECT id, project_id, module_id, submodule_id, parent_task_id,
       title, status, priority, note, sort_order
FROM task;

DROP TABLE task;
ALTER TABLE task_v2 RENAME TO task;

CREATE INDEX idx_task_project ON task(project_id);
CREATE INDEX idx_task_module  ON task(module_id);
CREATE INDEX idx_task_parent  ON task(parent_task_id);
"#;

/// v3: lower historical subtasks whose priority is above their parent's to the parent's
/// level. From this version on the rule is enforced by `domain::set_priority`; the migration
/// only brings historical data back into a valid state.
const V3_CLAMP_SUBTASK_PRIORITY: &str = r#"
UPDATE task
SET priority = (
  SELECT parent.priority
  FROM task AS parent
  WHERE parent.id = task.parent_task_id
)
WHERE parent_task_id IS NOT NULL
  AND (
    CASE priority
      WHEN 'high' THEN 3
      WHEN 'mid'  THEN 2
      ELSE 1
    END
  ) > (
    SELECT CASE parent.priority
             WHEN 'high' THEN 3
             WHEN 'mid'  THEN 2
             ELSE 1
           END
    FROM task AS parent
    WHERE parent.id = task.parent_task_id
  );
"#;

/// v4: tasks gain start / end times. Both columns are nullable and store local time
/// strings `YYYY-MM-DDTHH:MM` (same format as HTML `datetime-local`, comparable as text).
/// After the upgrade, existing tasks have NULL in both columns, meaning unset.
const V4_TASK_SCHEDULE: &str = r#"
ALTER TABLE task ADD COLUMN start_at TEXT;
ALTER TABLE task ADD COLUMN end_at   TEXT;
"#;

/// v5: tasks gain a creation time. Nullable: tasks from before the upgrade have no known
/// creation moment, so they stay NULL. New tasks created afterwards are written by
/// `domain::create_task` with local time `YYYY-MM-DDTHH:MM`.
const V5_TASK_CREATED_AT: &str = r#"
ALTER TABLE task ADD COLUMN created_at TEXT;
"#;

/// How many migrations the database currently has applied. `0` means a brand-new database.
pub fn current_version(conn: &Connection) -> Res<i64> {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|e| {
            Message::new(MessageKey::StartupReadVersionFailed).param("detail", e.to_string())
        })
}

/// Total number of migrations this program supports.
pub fn target_version() -> i64 {
    MIGRATIONS.len() as i64
}

/// Bring the database up to the latest version. Does nothing if it is already latest.
pub fn migrate(conn: &mut Connection) -> Res<()> {
    let current = current_version(conn)?;
    let target = target_version();

    if current > target {
        // The database is newer than the program: likely a downgrade. Writing further
        // would damage a structure of an unknown version.
        return Err(Message::new(MessageKey::StartupNewerDatabase)
            .param("current", current)
            .param("target", target));
    }
    if current == target {
        return Ok(());
    }

    // PRAGMA foreign_keys is a no-op inside a transaction, so it must be disabled before
    // opening one.
    conn.pragma_update(None, "foreign_keys", false)
        .map_err(|e| {
            Message::new(MessageKey::StartupDisableForeignKeysFailed)
                .param("detail", e.to_string())
        })?;

    let applied = apply_all(conn, current as usize);

    // Restore foreign keys whether or not this succeeds; never leave the connection
    // half-configured.
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(|e| {
            Message::new(MessageKey::StartupEnableForeignKeysFailed).param("detail", e.to_string())
        })?;
    applied
}

/// Wrap the whole thing in a single transaction: migrations either all land or none do.
fn apply_all(conn: &mut Connection, from: usize) -> Res<()> {
    let tx = conn.transaction().map_err(|e| {
        Message::new(MessageKey::StartupBeginTransactionFailed).param("detail", e.to_string())
    })?;

    for (i, sql) in MIGRATIONS.iter().enumerate().skip(from) {
        tx.execute_batch(sql).map_err(|e| {
            // SQLite errors include the whole failed SQL; keep only the first line for
            // the user.
            let message = e.to_string();
            let first = message.lines().next().unwrap_or(&message);
            // The SQL after "table X already exists in" has been truncated, so drop the
            // trailing " in" as well.
            let first = first.strip_suffix(" in").unwrap_or(first);
            Message::new(MessageKey::StartupMigrationFailed)
                .param("version", i + 1)
                .param("detail", first)
        })?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)
            .map_err(|e| {
                Message::new(MessageKey::StartupWriteVersionFailed)
                    .param("version", i + 1)
                    .param("detail", e.to_string())
            })?;
    }

    // Validation must happen before commit: on failure user_version rolls back with it,
    // so the next startup fails again and refuses to start. If we validated after commit,
    // the next startup would think the migration finished and let it through.
    check_foreign_keys(&tx)?;

    tx.commit().map_err(|e| {
        Message::new(MessageKey::StartupCommitFailed).param("detail", e.to_string())
    })
}

/// Run a full-database foreign key check after commit: a wrong cascade rule must blow up
/// here, not during some later delete.
fn check_foreign_keys(conn: &Connection) -> Res<()> {
    let mut stmt = conn.prepare("PRAGMA foreign_key_check").map_err(|e| {
        Message::new(MessageKey::StartupForeignKeyCheckFailed).param("detail", e.to_string())
    })?;
    let mut rows = stmt.query([]).map_err(|e| {
        Message::new(MessageKey::StartupForeignKeyCheckFailed).param("detail", e.to_string())
    })?;

    let mut violations = 0;
    while rows
        .next()
        .map_err(|e| {
            Message::new(MessageKey::StartupForeignKeyCheckReadFailed)
                .param("detail", e.to_string())
        })?
        .is_some()
    {
        violations += 1;
    }

    if violations > 0 {
        return Err(Message::new(MessageKey::StartupForeignKeyViolations)
            .param("count", violations as usize));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run migrations on an in-memory database and confirm the schema lands and the
    /// version reaches the latest.
    #[test]
    fn migrate_fresh_database() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();

        migrate(&mut conn).unwrap();

        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);

        for table in ["project", "module", "submodule", "task"] {
            let exists: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(exists, "table {table} was not created");
        }
    }

    /// Running it again is a no-op: no error, no scripts re-executed.
    #[test]
    fn migrate_is_idempotent() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        migrate(&mut conn).unwrap();
    }

    /// Refuse to start when the database is newer than the program, instead of writing on
    /// regardless.
    #[test]
    fn refuses_newer_database() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", MIGRATIONS.len() as i64 + 1)
            .unwrap();
        assert!(migrate(&mut conn).is_err());
    }

    fn count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    /// v2 only rebuilds the task table: old data is preserved as-is, and classification
    /// foreign keys become CASCADE.
    #[test]
    fn v2_upgrades_existing_data_and_cascades() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        conn.execute_batch(V1_INITIAL_SCHEMA).unwrap();
        conn.pragma_update(None, "user_version", 1i64).unwrap();
        conn.execute_batch(
            "INSERT INTO project (id, name) VALUES (1, '项目');
             INSERT INTO module (id, project_id, name) VALUES (1, 1, '模块');
             INSERT INTO submodule (id, module_id, name) VALUES (1, 1, '子模块');
             INSERT INTO task (id, project_id, module_id, submodule_id, title)
                    VALUES (1, 1, 1, 1, '父任务');
             INSERT INTO task (id, project_id, module_id, submodule_id, parent_task_id, title)
                    VALUES (2, 1, 1, 1, 1, '子任务');
             INSERT INTO task (id, project_id, title) VALUES (3, 1, '未归类');",
        )
        .unwrap();

        migrate(&mut conn).unwrap();

        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
        assert_eq!(count(&conn, "task"), 3);
        assert_eq!(count(&conn, "submodule"), 1);

        let actions: Vec<(String, String)> = {
            let mut stmt = conn.prepare("PRAGMA foreign_key_list(task)").unwrap();
            let rows = stmt
                .query_map([], |row| Ok((row.get(3)?, row.get(6)?)))
                .unwrap();
            rows.map(|row| row.unwrap()).collect()
        };
        for column in ["module_id", "submodule_id"] {
            assert!(
                actions
                    .iter()
                    .any(|(from, on_delete)| from == column && on_delete == "CASCADE"),
                "{column} was not switched to CASCADE: {actions:?}"
            );
        }

        conn.execute("DELETE FROM module WHERE id = 1", []).unwrap();
        assert_eq!(count(&conn, "task"), 1, "tasks in the module should be deleted with it");
        assert_eq!(count(&conn, "submodule"), 0);
        check_foreign_keys(&conn).unwrap();
    }

    /// v3 lowers historical subtasks with a higher priority down to the parent's level;
    /// valid subtasks are untouched.
    #[test]
    fn v3_clamps_children_to_parent_priority() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        conn.execute_batch(V1_INITIAL_SCHEMA).unwrap();
        conn.pragma_update(None, "user_version", 1i64).unwrap();
        conn.execute_batch(
            "INSERT INTO task (id, title, priority) VALUES (1, '低父', 'low');
             INSERT INTO task (id, parent_task_id, title, priority)
                    VALUES (2, 1, '高子', 'high');
             INSERT INTO task (id, title, priority) VALUES (3, '高父', 'high');
             INSERT INTO task (id, parent_task_id, title, priority)
                    VALUES (4, 3, '低子', 'low');
             INSERT INTO task (id, title, priority) VALUES (5, '中父', 'mid');
             INSERT INTO task (id, parent_task_id, title, priority)
                    VALUES (6, 5, '中子', 'mid');",
        )
        .unwrap();

        migrate(&mut conn).unwrap();

        let priorities: Vec<(i64, String)> = {
            let mut stmt = conn
                .prepare("SELECT id, priority FROM task ORDER BY id")
                .unwrap();
            let rows = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap();
            rows.map(|row| row.unwrap()).collect()
        };
        assert_eq!(
            priorities,
            vec![
                (1, "low".to_string()),
                (2, "low".to_string()),
                (3, "high".to_string()),
                (4, "low".to_string()),
                (5, "mid".to_string()),
                (6, "mid".to_string()),
            ]
        );
    }

    /// v4 only adds two columns: old tasks are preserved, and times default to unset.
    #[test]
    fn v4_adds_empty_schedule_columns() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        conn.execute_batch(V1_INITIAL_SCHEMA).unwrap();
        conn.pragma_update(None, "user_version", 1i64).unwrap();
        conn.execute_batch("INSERT INTO task (id, title) VALUES (1, '旧任务');")
            .unwrap();

        migrate(&mut conn).unwrap();

        let schedule: (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT start_at, end_at FROM task WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(schedule, (None, None), "upgraded tasks should have unset times");
    }

    /// v5 only adds the created_at column: old tasks have an unknown creation time and stay
    /// NULL.
    #[test]
    fn v5_adds_empty_created_at() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        conn.execute_batch(V1_INITIAL_SCHEMA).unwrap();
        conn.pragma_update(None, "user_version", 1i64).unwrap();
        conn.execute_batch("INSERT INTO task (id, title) VALUES (1, '旧任务');")
            .unwrap();

        migrate(&mut conn).unwrap();

        let created_at: Option<String> = conn
            .query_row("SELECT created_at FROM task WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(created_at, None, "tasks from before the upgrade must not fake a creation time");
    }

    /// Foreign key validation happens inside the transaction: with dangling references the
    /// whole thing rolls back, user_version is unchanged, and no new tables are left behind.
    #[test]
    fn foreign_key_check_failure_rolls_back() {
        let mut conn = Connection::open_in_memory().unwrap();
        // Bundled SQLite has foreign keys on by default; disable them first to construct
        // dirty historical data.
        conn.pragma_update(None, "foreign_keys", false).unwrap();
        conn.execute_batch(V1_INITIAL_SCHEMA).unwrap();
        conn.pragma_update(None, "user_version", 1i64).unwrap();
        conn.execute_batch("INSERT INTO task (id, module_id, title) VALUES (1, 999, '悬空模块');")
            .unwrap();

        let err = migrate(&mut conn).unwrap_err();
        assert_eq!(err.key, MessageKey::StartupForeignKeyViolations);
        assert_eq!(
            err.params.get("count"),
            Some(&crate::message::MessageParam::Number(1))
        );
        assert_eq!(
            current_version(&conn).unwrap(),
            1,
            "failure must roll back user_version"
        );

        let v2_exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='task_v2')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!v2_exists, "a failed transaction must not leave task_v2 behind");
    }
}
