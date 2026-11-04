//! 存储层：库文件位置、连接、连接级 PRAGMA、迁移前备份。
//!
//! 只有一个入口 [`open`]，因为它顺带备份与迁移——不想让别处拿到一个
//! "还没迁移完"的连接。规则不在这里，在 `domain.rs`。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use tauri::{AppHandle, Manager};

use crate::migrations;

/// 应用数据子目录（Windows 下即 `%APPDATA%/todolist`）。
pub const APP_DIR: &str = "todolist";

/// 库文件名。
pub const DB_FILE: &str = "tasks.sqlite";

/// 迁移前备份保留的份数（含刚生成的那一份）。
const BACKUP_KEEP: usize = 5;

/// 交给 Tauri 托管的连接句柄。
///
/// 单连接 + `Mutex`：本应用是单窗口单用户，连接池带来的并发能力用不上，
/// 反而多一层"事务可能在另一条连接上"的迷惑。
pub struct Db(Mutex<Connection>);

impl Db {
    /// 定位库文件、建立连接、跑完迁移。
    pub fn open_in_app_data(app: &AppHandle) -> Result<Self, String> {
        let path = db_path(app)?;
        let conn = open(&path)?;
        Ok(Db(Mutex::new(conn)))
    }

    /// 取连接。中毒（上一个持有者 panic）时继续用：SQLite 自身状态不会因此损坏，
    /// 而"直接失败"只会把一个已经恢复的可能性变成永久故障。
    pub fn conn(&self) -> MutexGuard<'_, Connection> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 开发期删库重建：关文件连接 → 删库 → 重新建库并跑迁移。
    ///
    /// 整段持锁，别的命令不会看到中间那个临时的内存连接。主文件删不掉时
    /// 把原库重新接回去，让应用还能继续用。
    ///
    /// 只清活库与它的边车文件；`.bak-*` 备份保留——它们正是出问题时的恢复副本。
    #[cfg(debug_assertions)]
    pub fn reset(&self, path: &Path) -> Result<(), String> {
        let mut guard = self.conn();

        // Windows 上必须先放掉文件句柄，否则删除会失败。
        let placeholder =
            Connection::open_in_memory().map_err(|e| format!("无法准备重置数据库：{e}"))?;
        let old = std::mem::replace(&mut *guard, placeholder);
        drop(old);

        match remove_database_files(path) {
            Ok(()) => {
                *guard = open(path)?;
                Ok(())
            }
            Err(error) => {
                // 原库还在：重新接上，别让应用停在临时连接上。
                match open(path) {
                    Ok(conn) => *guard = conn,
                    Err(reopen) => {
                        return Err(format!("{error}；重新打开数据库也失败：{reopen}"));
                    }
                }
                Err(error)
            }
        }
    }
}

/// `%APPDATA%/todolist/tasks.sqlite`，目录不存在就建。
pub fn db_path(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app
        .path()
        .data_dir()
        .map_err(|e| format!("无法定位应用数据目录：{e}"))?;
    let dir = base.join(APP_DIR);
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建目录 {}：{e}", dir.display()))?;
    Ok(dir.join(DB_FILE))
}

/// 打开连接 → 外键 ON → 迁移前备份（仅旧库升级）→ 迁移。
pub fn open(path: &Path) -> Result<Connection, String> {
    let mut conn =
        Connection::open(path).map_err(|e| format!("无法打开数据库 {}：{e}", path.display()))?;

    // SQLite 默认关闭外键，每个连接都要显式打开，否则 ON DELETE 级联静默失效。
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(|e| format!("无法开启外键约束：{e}"))?;

    let current = migrations::current_version(&conn)?;
    let target = migrations::target_version();

    // 新库（0）没有数据可保护；同版本或更高版本也不会写，所以都不备份。
    let backup = if 0 < current && current < target {
        Some(backup_before_migration(path, current)?)
    } else {
        None
    };

    // 备份路径放在最前面：它是用户最需要的恢复信息，不该被长错误挤到滚动区底部。
    migrations::migrate(&mut conn).map_err(|error| match &backup {
        Some(backup) => format!("迁移前的备份：{}\n\n{error}", backup.display()),
        None => error,
    })?;

    Ok(conn)
}

/// 把迁移前的库复制成 `tasks.sqlite.bak-v<旧版本>-<时间戳>`。
///
/// 此刻连接空闲、没有事务，库用的是默认 rollback journal，文件本身就是
/// 一致快照，直接复制即可。备份失败就不迁移——这是本阶段的安全底线。
fn backup_before_migration(path: &Path, from_version: i64) -> Result<PathBuf, String> {
    let target = backup_path(path, from_version);

    if let Err(error) = fs::copy(path, &target) {
        // 半截的备份比没有备份更危险：清掉再报错。
        let _ = fs::remove_file(&target);
        return Err(format!(
            "迁移前备份失败，已停止迁移：无法把 {} 复制到 {}：{error}",
            path.display(),
            target.display()
        ));
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

    // 同一毫秒内连续备份（测试里会发生）也要拿到不同的名字。
    let mut stamp = now_millis();
    loop {
        let candidate = dir.join(format!("{file_name}.bak-v{from_version}-{stamp}"));
        if !candidate.exists() {
            return candidate;
        }
        stamp += 1;
    }
}

/// 只清理本程序生成的 `tasks.sqlite.bak-v*`；手工备份（如 `.bak-import-*`）不碰。
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

    // 删除失败不阻断启动：多留几份备份不会破坏数据。
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

/// 删除活库与它的边车文件。主文件的 `NotFound` 视为已经删掉了。
#[cfg(debug_assertions)]
fn remove_database_files(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("无法删除数据库 {}：{error}", path.display())),
    }

    // 边车属于同一个库；删不掉也不阻止重建，SQLite 会忽略孤立的边车。
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

    /// 每个测试一个临时目录；连接先于目录释放（声明顺序决定 Drop 顺序）。
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir()
                .join(format!("todolist-test-{}-{tag}-{n}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            // 句柄可能还没完全释放；清理失败不影响测试结论。
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// 建一个 v1 库并写入一条项目/任务，然后关掉连接。
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
        assert_eq!(found.len(), 1, "旧库升级应生成一份备份");

        let backup = Connection::open(&found[0]).unwrap();
        assert_eq!(migrations::current_version(&backup).unwrap(), 1);
        let projects: i64 = backup
            .query_row("SELECT COUNT(*) FROM project", [], |row| row.get(0))
            .unwrap();
        assert_eq!(projects, 1, "备份里应保留迁移前的数据");
        assert!(decoy.exists(), "手工备份不能被清理");
    }

    #[test]
    fn no_backup_for_fresh_database() {
        let dir = TempDir::new("fresh");
        let path = dir.path().join(DB_FILE);

        drop(open(&path).unwrap());

        assert!(backups(dir.path()).is_empty(), "新库没有数据可保护");
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
            // 让 v2 的 CREATE TABLE task_v2 撞名，迁移必然失败。
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE task_v2 (x INTEGER);")
                .unwrap();
        }

        let error = open(&path).unwrap_err();
        assert!(error.contains("迁移前的备份"), "{error}");
        assert_eq!(backups(dir.path()).len(), 1);

        let conn = Connection::open(&path).unwrap();
        assert_eq!(
            migrations::current_version(&conn).unwrap(),
            1,
            "失败的事务不能推进 user_version"
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
        assert!(path.exists(), "重置后应重新建出库文件");
    }
}
