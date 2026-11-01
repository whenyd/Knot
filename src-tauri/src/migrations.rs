//! 迁移：`MIGRATIONS` 是全部 schema 变更的顺序列表，`user_version` 即"已应用到第几条"。
//!
//! 只写前进，不写 down。回滚靠迁移前的文件备份，而不是一条长期没人执行的逆向脚本。

use rusqlite::Connection;

/// 顺序执行的迁移脚本。索引 `i` 的脚本把库从版本 `i` 推到 `i + 1`。
///
/// 追加新条目即可；已发布的条目一个字都不要改。
pub const MIGRATIONS: &[&str] = &[V1_INITIAL_SCHEMA];

/// v1：项目 / 模块 / 子模块 / 任务，两层分类 + 一层子任务。
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
  project_id     INTEGER REFERENCES project(id) ON DELETE CASCADE,  -- NULL = 「待办」
  module_id      INTEGER REFERENCES module(id) ON DELETE SET NULL,
  submodule_id   INTEGER REFERENCES submodule(id) ON DELETE SET NULL,
  parent_task_id INTEGER REFERENCES task(id) ON DELETE CASCADE,     -- NULL = 顶层任务
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

/// 把库推到最新版本。已经是最新就什么都不做。
pub fn migrate(conn: &mut Connection) -> Result<(), String> {
    let current: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|e| format!("无法读取 user_version：{e}"))?;
    let target = MIGRATIONS.len() as i64;

    if current > target {
        // 库比程序新：多半是降级运行。继续写会破坏未知版本的结构。
        return Err(format!(
            "数据库版本 {current} 高于本程序支持的 {target}，拒绝启动以免损坏数据"
        ));
    }
    if current == target {
        return Ok(());
    }

    // PRAGMA foreign_keys 在事务内是 no-op，所以必须在开事务之前关。
    conn.pragma_update(None, "foreign_keys", false)
        .map_err(|e| format!("无法临时关闭外键约束：{e}"))?;

    let applied = apply_all(conn, current as usize);

    // 无论成功失败都先把外键恢复，别把连接留在半配置状态。
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(|e| format!("无法恢复外键约束：{e}"))?;
    applied?;

    check_foreign_keys(conn)
}

/// 整段包在单个事务里：迁移要么全到，要么一点不动。
fn apply_all(conn: &mut Connection, from: usize) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("无法开启迁移事务：{e}"))?;

    for (i, sql) in MIGRATIONS.iter().enumerate().skip(from) {
        tx.execute_batch(sql)
            .map_err(|e| format!("迁移到版本 {} 失败：{e}", i + 1))?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)
            .map_err(|e| format!("迁移到版本 {} 后写 user_version 失败：{e}", i + 1))?;
    }

    tx.commit()
        .map_err(|e| format!("提交迁移事务失败：{e}"))
}

/// 提交后跑一次全库外键检查：级联规则写错了，要在这里就炸，而不是等到某次删除。
fn check_foreign_keys(conn: &Connection) -> Result<(), String> {
    let mut stmt = conn
        .prepare("PRAGMA foreign_key_check")
        .map_err(|e| format!("无法执行外键校验：{e}"))?;
    let mut rows = stmt
        .query([])
        .map_err(|e| format!("无法执行外键校验：{e}"))?;

    let mut violations = 0;
    while rows
        .next()
        .map_err(|e| format!("无法读取外键校验结果：{e}"))?
        .is_some()
    {
        violations += 1;
    }

    if violations > 0 {
        return Err(format!("迁移后外键校验失败：{violations} 条悬空引用"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 内存库跑一遍迁移，确认 schema 落地且版本号推进到最新。
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
            assert!(exists, "表 {table} 未创建");
        }
    }

    /// 再跑一次是 no-op，不报错、不重复执行脚本。
    #[test]
    fn migrate_is_idempotent() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        migrate(&mut conn).unwrap();
    }

    /// 库比程序新时拒绝启动，而不是硬着头皮写下去。
    #[test]
    fn refuses_newer_database() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", MIGRATIONS.len() as i64 + 1)
            .unwrap();
        assert!(migrate(&mut conn).is_err());
    }
}
