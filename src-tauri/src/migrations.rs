//! 迁移：`MIGRATIONS` 是全部 schema 变更的顺序列表，`user_version` 即"已应用到第几条"。
//!
//! 只写前进，不写 down。回滚靠迁移前的文件备份，而不是一条长期没人执行的逆向脚本。

use rusqlite::Connection;

/// 顺序执行的迁移脚本。索引 `i` 的脚本把库从版本 `i` 推到 `i + 1`。
///
/// 追加新条目即可；已发布的条目一个字都不要改。
pub const MIGRATIONS: &[&str] = &[
    V1_INITIAL_SCHEMA,
    V2_TASK_CLASSIFICATION_CASCADE,
    V3_CLAMP_SUBTASK_PRIORITY,
    V4_TASK_SCHEDULE,
    V5_TASK_CREATED_AT,
];

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
  project_id     INTEGER REFERENCES project(id) ON DELETE CASCADE,  -- NULL = 「我的待办」
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

/// v2：删除模块 / 子模块时，组内任务跟着删除，而不是退回归类。
///
/// SQLite 不能直接改外键动作，只能重建表。迁移期间外键已被关闭，
/// 所以可以整表拷贝；`parent_task_id` 的自引用会随表名一起更新。
const V2_TASK_CLASSIFICATION_CASCADE: &str = r#"
CREATE TABLE task_v2 (
  id             INTEGER PRIMARY KEY,
  project_id     INTEGER REFERENCES project(id) ON DELETE CASCADE,  -- NULL = 「我的待办」
  module_id      INTEGER REFERENCES module(id) ON DELETE CASCADE,
  submodule_id   INTEGER REFERENCES submodule(id) ON DELETE CASCADE,
  parent_task_id INTEGER REFERENCES task_v2(id) ON DELETE CASCADE,  -- NULL = 顶层任务
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

/// v3：把旧数据里高于父任务的子任务降到父任务的档位。
/// 规则从这一版起由 `domain::set_priority` 保证；迁移只负责把历史数据拉回合法状态。
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

/// v4：任务增加开始 / 结束时间。两列都可空，存本地时间字符串
/// `YYYY-MM-DDTHH:MM`（格式与 HTML `datetime-local` 一致，按文本可比较）。
/// 已有任务升级后两列为 NULL，表示未设置。
const V4_TASK_SCHEDULE: &str = r#"
ALTER TABLE task ADD COLUMN start_at TEXT;
ALTER TABLE task ADD COLUMN end_at   TEXT;
"#;

/// v5：任务增加创建时间。可空：升级前的任务不知道创建时刻，保持 NULL；
/// 之后新建的任务由 `domain::create_task` 写入本地时间 `YYYY-MM-DDTHH:MM`。
const V5_TASK_CREATED_AT: &str = r#"
ALTER TABLE task ADD COLUMN created_at TEXT;
"#;

/// 库当前已应用到第几条迁移。`0` 表示全新库。
pub fn current_version(conn: &Connection) -> Result<i64, String> {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|e| format!("无法读取 user_version：{e}"))
}

/// 本程序支持的迁移总数。
pub fn target_version() -> i64 {
    MIGRATIONS.len() as i64
}

/// 把库推到最新版本。已经是最新就什么都不做。
pub fn migrate(conn: &mut Connection) -> Result<(), String> {
    let current = current_version(conn)?;
    let target = target_version();

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
    applied
}

/// 整段包在单个事务里：迁移要么全到，要么一点不动。
fn apply_all(conn: &mut Connection, from: usize) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("无法开启迁移事务：{e}"))?;

    for (i, sql) in MIGRATIONS.iter().enumerate().skip(from) {
        tx.execute_batch(sql).map_err(|e| {
            // SQLite 的报错会把整段失败的 SQL 也带上；给用户只留第一行。
            let message = e.to_string();
            let first = message.lines().next().unwrap_or(&message);
            // 「table X already exists in」后面的 SQL 已被截掉，收尾的 " in" 也去掉。
            let first = first.strip_suffix(" in").unwrap_or(first);
            format!("迁移到版本 {} 失败：{first}", i + 1)
        })?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)
            .map_err(|e| format!("迁移到版本 {} 后写 user_version 失败：{e}", i + 1))?;
    }

    // 校验必须在提交之前：失败时连 user_version 一起回滚，下次启动还会重新
    // 失败并拒绝启动；提交后再校验的话，下一次启动会以为迁移已完成而放行。
    check_foreign_keys(&tx)?;

    tx.commit().map_err(|e| format!("提交迁移事务失败：{e}"))
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
        return Err(format!("迁移校验失败（已回滚）：{violations} 条悬空引用"));
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

    fn count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    /// v2 只重建 task 表：老数据原样保留，分类外键改成级联删除。
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
                "{column} 没有改成级联删除：{actions:?}"
            );
        }

        conn.execute("DELETE FROM module WHERE id = 1", []).unwrap();
        assert_eq!(count(&conn, "task"), 1, "组内任务应随模块一起删除");
        assert_eq!(count(&conn, "submodule"), 0);
        check_foreign_keys(&conn).unwrap();
    }

    /// v3 把旧数据里高于父任务的子任务降到父任务档位；合法的子任务不动。
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

    /// v4 只加两列：老任务保留，时间默认未设置。
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
        assert_eq!(schedule, (None, None), "升级后的任务时间应为未设置");
    }

    /// v5 只加创建时间列：老任务不知道创建时刻，保持 NULL。
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
        assert_eq!(created_at, None, "升级前的任务不伪造创建时间");
    }

    /// 外键校验在事务内：有悬空引用时整段回滚，user_version 不变、新表不留下。
    #[test]
    fn foreign_key_check_failure_rolls_back() {
        let mut conn = Connection::open_in_memory().unwrap();
        // bundled SQLite 默认开着外键；构造历史脏数据前先关掉。
        conn.pragma_update(None, "foreign_keys", false).unwrap();
        conn.execute_batch(V1_INITIAL_SCHEMA).unwrap();
        conn.pragma_update(None, "user_version", 1i64).unwrap();
        conn.execute_batch("INSERT INTO task (id, module_id, title) VALUES (1, 999, '悬空模块');")
            .unwrap();

        let err = migrate(&mut conn).unwrap_err();
        assert!(err.contains("悬空引用"), "{err}");
        assert_eq!(
            current_version(&conn).unwrap(),
            1,
            "失败必须回滚 user_version"
        );

        let v2_exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='task_v2')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!v2_exists, "失败的事务不应留下 task_v2");
    }
}
