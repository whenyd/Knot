//! Storage layer: database file location, connection, connection-level PRAGMAs, and
//! pre-migration backups.
//!
//! There is only one entry point, [`open`], because it also handles backup and migration—we
//! do not want anything else to get a connection that is "not migrated yet". Rules do not
//! live here; they live in `domain.rs`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::message::{Message, MessageKey};
use crate::migrations;

/// The reason the app could not open its database, plus the backup that was
/// taken before the failed migration (if one was taken).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupFailure {
    pub error: Message,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
}

impl From<Message> for StartupFailure {
    fn from(error: Message) -> Self {
        Self {
            error,
            backup_path: None,
        }
    }
}

/// Application data subdirectory (on Windows this is `%APPDATA%/knot`).
pub const APP_DIR: &str = "knot";

/// Database file name.
pub const DB_FILE: &str = "tasks.sqlite";

/// Number of pre-migration backups to keep (including the one just created).
const BACKUP_KEEP: usize = 5;

/// Connection handle managed by Tauri.
///
/// Single connection + `Mutex`: this app is single-window and single-user, so a connection
/// pool's concurrency buys us nothing and only adds the confusion of "the transaction might
/// be on another connection".
pub struct Db(Mutex<Connection>);

impl Db {
    /// Locate the database file, open the connection, and run migrations.
    pub fn open_in_app_data(app: &AppHandle) -> Result<Self, StartupFailure> {
        let path = db_path(app)?;
        let conn = open(&path)?;
        Ok(Db(Mutex::new(conn)))
    }

    /// Borrow the connection. If the mutex is poisoned (a previous holder panicked), keep
    /// using it: SQLite's own state is not corrupted by that, and failing outright would turn
    /// a recoverable situation into a permanent fault.
    pub fn conn(&self) -> MutexGuard<'_, Connection> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Debug-time database reset: close the file connection → delete the database →
    /// recreate it and run migrations.
    ///
    /// The lock is held throughout so other commands never see the temporary in-memory
    /// connection in between. If the main file cannot be deleted, reconnect the original
    /// database so the app remains usable.
    ///
    /// Only the live database and its sidecar files are removed; `.bak-*` backups are
    /// kept—they are exactly the recovery copies you need when something goes wrong.
    #[cfg(debug_assertions)]
    pub fn reset(&self, path: &Path) -> Result<(), Message> {
        let mut guard = self.conn();

        // On Windows the file handle must be released first, otherwise deletion fails.
        let placeholder = Connection::open_in_memory().map_err(|e| {
            Message::new(MessageKey::DbResetPrepareFailed).param("detail", e.to_string())
        })?;
        let old = std::mem::replace(&mut *guard, placeholder);
        drop(old);

        match remove_database_files(path) {
            Ok(()) => {
                *guard = open(path).map_err(|failure| failure.error)?;
                Ok(())
            }
            Err(error) => {
                // The original database is still there: reconnect so the app does not
                // get stuck on the temporary connection.
                match open(path) {
                    Ok(conn) => *guard = conn,
                    Err(reopen) => {
                        return Err(Message::new(MessageKey::DbReopenFailed)
                            .param("detail", reopen.error));
                    }
                }
                Err(error)
            }
        }
    }
}

/// `%APPDATA%/knot/tasks.sqlite`, creating the directory if it does not exist.
pub fn db_path(app: &AppHandle) -> Result<PathBuf, Message> {
    let base = app.path().data_dir().map_err(|e| {
        Message::new(MessageKey::DbDataDirFailed).param("detail", e.to_string())
    })?;
    let dir = base.join(APP_DIR);
    fs::create_dir_all(&dir).map_err(|e| {
        Message::new(MessageKey::DbCreateDirFailed)
            .param("path", dir.display().to_string())
            .param("detail", e.to_string())
    })?;
    Ok(dir.join(DB_FILE))
}

/// Open connection → foreign keys ON → pre-migration backup (only when upgrading an old
/// database) → migrate.
pub fn open(path: &Path) -> Result<Connection, StartupFailure> {
    let mut conn = Connection::open(path).map_err(|e| {
        Message::new(MessageKey::DbOpenFailed)
            .param("path", path.display().to_string())
            .param("detail", e.to_string())
    })?;

    // SQLite disables foreign keys by default; every connection must enable them
    // explicitly, or ON DELETE cascades silently do nothing.
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(|e| {
            Message::new(MessageKey::DbForeignKeysFailed).param("detail", e.to_string())
        })?;

    let current = migrations::current_version(&conn)?;
    let target = migrations::target_version();

    // A fresh database (0) has no data to protect; the same or a newer version will not be
    // written, so neither is backed up.
    let backup = if 0 < current && current < target {
        Some(backup_before_migration(path, current)?)
    } else {
        None
    };

    migrations::migrate(&mut conn).map_err(|error| StartupFailure {
        error,
        backup_path: backup.map(|path| path.display().to_string()),
    })?;

    Ok(conn)
}

/// Copy the pre-migration database to `tasks.sqlite.bak-v<old version>-<timestamp>`.
///
/// The connection is idle and there is no transaction at this point, and the database uses
/// the default rollback journal, so the file itself is a consistent snapshot and can be
/// copied directly. If the backup fails, skip migration—this is the safety floor for this
/// phase.
fn backup_before_migration(path: &Path, from_version: i64) -> Result<PathBuf, Message> {
    let target = backup_path(path, from_version);

    if let Err(error) = fs::copy(path, &target) {
        // A half-written backup is more dangerous than none: remove it before reporting
        // the error.
        let _ = fs::remove_file(&target);
        return Err(Message::new(MessageKey::DbBackupCopyFailed)
            .param("from", path.display().to_string())
            .param("to", target.display().to_string())
            .param("detail", error.to_string()));
    }

    prune_backups(path);
    Ok(target)
}

fn backup_path(path: &Path, from_version: i64) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| DB_FILE.to_string());
    let dir = path.parent().unwrap_or_else(|| Path::new("."));

    // Consecutive backups within the same millisecond (which happens in tests) still
    // need distinct names.
    let mut stamp = now_millis();
    loop {
        let candidate = dir.join(format!("{file_name}.bak-v{from_version}-{stamp}"));
        if !candidate.exists() {
            return candidate;
        }
        stamp += 1;
    }
}

/// Only remove `tasks.sqlite.bak-v*` files created by this program; manual backups
/// (e.g. `.bak-import-*`) are left alone.
fn prune_backups(path: &Path) {
    let Some(dir) = path.parent() else { return };
    let Some(file_name) = path.file_name() else {
        return;
    };
    let prefix = format!("{}.bak-v", file_name.to_string_lossy());

    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut backups: Vec<(u128, PathBuf)> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let stamp = name
                .strip_prefix(&prefix)?
                .rsplit_once('-')?
                .1
                .parse()
                .ok()?;
            Some((stamp, entry.path()))
        })
        .collect();

    if backups.len() <= BACKUP_KEEP {
        return;
    }

    // Deletion failures must not block startup: leaving a few extra backups cannot
    // corrupt data.
    backups.sort_by_key(|(stamp, _)| *stamp);
    let remove_count = backups.len() - BACKUP_KEEP;
    for (_, old) in backups.into_iter().take(remove_count) {
        let _ = fs::remove_file(old);
    }
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

/// Delete the live database and its sidecar files. A `NotFound` on the main file counts as
/// already deleted.
#[cfg(debug_assertions)]
fn remove_database_files(path: &Path) -> Result<(), Message> {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(Message::new(MessageKey::DbDeleteFailed)
                .param("path", path.display().to_string())
                .param("detail", error.to_string()));
        }
    }

    // Sidecars belong to the same database; failing to delete them does not block
    // recreation, and SQLite ignores orphaned sidecars.
    for suffix in ["-journal", "-wal", "-shm"] {
        let _ = fs::remove_file(sidecar_path(path, suffix));
    }
    Ok(())
}

#[cfg(debug_assertions)]
fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// One temp directory per test; the connection is dropped before the directory
    /// (declaration order determines Drop order).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir()
                .join(format!("knot-test-{}-{tag}-{n}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            // Handles may not be fully released yet; cleanup failures do not affect
            // test results.
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Create a v1 database with one project/task, then close the connection.
    fn make_v1_db(dir: &Path) -> PathBuf {
        let path = dir.join(DB_FILE);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(migrations::MIGRATIONS[0]).unwrap();
        conn.pragma_update(None, "user_version", 1i64).unwrap();
        conn.execute_batch(
            "INSERT INTO project (id, name) VALUES (1, '项目');
             INSERT INTO task (id, project_id, title) VALUES (1, 1, '任务');",
        )
        .unwrap();
        path
    }

    fn backups(dir: &Path) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = fs::read_dir(dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().starts_with("tasks.sqlite.bak-v"))
                    .unwrap_or(false)
            })
            .collect();
        found.sort();
        found
    }

    #[test]
    fn backup_created_when_upgrading() {
        let dir = TempDir::new("upgrade");
        let path = make_v1_db(dir.path());
        let decoy = dir.path().join("tasks.sqlite.bak-import-20261103074018");
        fs::write(&decoy, b"manual").unwrap();

        drop(open(&path).unwrap());

        let found = backups(dir.path());
        assert_eq!(found.len(), 1, "upgrading an old database should create one backup");

        let backup = Connection::open(&found[0]).unwrap();
        assert_eq!(migrations::current_version(&backup).unwrap(), 1);
        let projects: i64 = backup
            .query_row("SELECT COUNT(*) FROM project", [], |row| row.get(0))
            .unwrap();
        assert_eq!(projects, 1, "the backup should preserve pre-migration data");
        assert!(decoy.exists(), "manual backups must not be pruned");
    }

    #[test]
    fn no_backup_for_fresh_database() {
        let dir = TempDir::new("fresh");
        let path = dir.path().join(DB_FILE);

        drop(open(&path).unwrap());

        assert!(backups(dir.path()).is_empty(), "a fresh database has no data to protect");
    }

    #[test]
    fn keeps_only_five_backups() {
        let dir = TempDir::new("prune");
        let path = make_v1_db(dir.path());

        for _ in 0..6 {
            backup_before_migration(&path, 1).unwrap();
        }

        assert_eq!(backups(dir.path()).len(), BACKUP_KEEP);
    }

    #[test]
    fn migration_failure_reports_backup() {
        let dir = TempDir::new("failure");
        let path = make_v1_db(dir.path());
        {
            // Make v2's CREATE TABLE task_v2 collide by name so migration must fail.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE task_v2 (x INTEGER);")
                .unwrap();
        }

        let failure = open(&path).unwrap_err();
        assert_eq!(failure.error.key, MessageKey::StartupMigrationFailed);
        assert!(
            failure.backup_path.is_some(),
            "a failed migration must report its backup"
        );
        assert_eq!(backups(dir.path()).len(), 1);

        let conn = Connection::open(&path).unwrap();
        assert_eq!(
            migrations::current_version(&conn).unwrap(),
            1,
            "a failed transaction must not advance user_version"
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    fn reset_deletes_and_recreates() {
        let dir = TempDir::new("reset");
        let path = dir.path().join(DB_FILE);
        let db = Db(Mutex::new(open(&path).unwrap()));
        {
            let conn = db.conn();
            conn.execute("INSERT INTO project (name) VALUES ('项目')", [])
                .unwrap();
        }

        db.reset(&path).unwrap();

        {
            let conn = db.conn();
            let projects: i64 = conn
                .query_row("SELECT COUNT(*) FROM project", [], |row| row.get(0))
                .unwrap();
            assert_eq!(projects, 0);
            assert_eq!(
                migrations::current_version(&conn).unwrap(),
                migrations::target_version()
            );
        }
        assert!(path.exists(), "reset should recreate the database file");
    }

    #[test]
    fn startup_failure_wire_shape_reports_message_and_backup() {
        let failure = StartupFailure {
            error: Message::new(MessageKey::StartupMigrationFailed)
                .param("version", 3i64)
                .param("detail", "boom"),
            backup_path: Some("C:\\tmp\\tasks.sqlite.bak-v2-1".to_string()),
        };
        let json = serde_json::to_value(&failure).unwrap();
        assert_eq!(json["error"]["key"], "startup.migrationFailed");
        assert_eq!(json["error"]["params"]["version"], 3);
        assert_eq!(json["error"]["params"]["detail"], "boom");
        assert_eq!(json["backupPath"], "C:\\tmp\\tasks.sqlite.bak-v2-1");

        let without_backup = StartupFailure {
            backup_path: None,
            ..failure
        };
        let json = serde_json::to_value(&without_backup).unwrap();
        assert!(
            json.get("backupPath").is_none(),
            "no backup means the field is omitted"
        );
    }
}
