//! 存储层：库文件位置、连接、连接级 PRAGMA。
//!
//! 只有一个入口 [`open`]，因为它顺带跑迁移——不想让别处拿到一个
//! "还没迁移完"的连接。规则不在这里，在 `domain.rs`。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use rusqlite::Connection;
use tauri::{AppHandle, Manager};

use crate::migrations;

/// 应用数据子目录（Windows 下即 `%APPDATA%/todolist`）。
pub const APP_DIR: &str = "todolist";

/// 库文件名。
pub const DB_FILE: &str = "tasks.sqlite";

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

/// 打开连接 → 外键 ON → 迁移。
pub fn open(path: &Path) -> Result<Connection, String> {
    let mut conn =
        Connection::open(path).map_err(|e| format!("无法打开数据库 {}：{e}", path.display()))?;

    // SQLite 默认关闭外键，每个连接都要显式打开，否则 ON DELETE 级联静默失效。
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(|e| format!("无法开启外键约束：{e}"))?;

    migrations::migrate(&mut conn)?;
    Ok(conn)
}
