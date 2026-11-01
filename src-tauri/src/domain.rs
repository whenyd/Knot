//! 领域层：唯一的写入口，也是唯一允许表达"规则"的地方。
//!
//! 命令层只做参数解包 + 转发；SQL 不重复表达这里的规则（不用触发器），
//! 否则同一套规则会有两个实现，必然漂移。

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// 左栏最上方那个虚拟项目。它不是 `project` 表里的一行——
/// "待办"由 `task.project_id IS NULL` 表达，这里只是它的显示名。
pub const TODO_NAME: &str = "待办";

type Res<T> = Result<T, String>;

// ---------------------------------------------------------------------------
// 类型
// ---------------------------------------------------------------------------

/// 任务状态。三态，格子里点一下就在这三个值之间循环。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Todo,
    Doing,
    Done,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Todo => "todo",
            Status::Doing => "doing",
            Status::Done => "done",
        }
    }

    fn from_db(raw: &str) -> Res<Self> {
        match raw {
            "todo" => Ok(Status::Todo),
            "doing" => Ok(Status::Doing),
            "done" => Ok(Status::Done),
            other => Err(format!("未知的状态值 {other:?}")),
        }
    }
}

/// 任务优先级。默认 `Mid`。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    High,
    Mid,
    Low,
}

impl Priority {
    pub fn as_str(self) -> &'static str {
        match self {
            Priority::High => "high",
            Priority::Mid => "mid",
            Priority::Low => "low",
        }
    }

    fn from_db(raw: &str) -> Res<Self> {
        match raw {
            "high" => Ok(Priority::High),
            "mid" => Ok(Priority::Mid),
            "low" => Ok(Priority::Low),
            other => Err(format!("未知的优先级值 {other:?}")),
        }
    }
}

/// 任务所在的容器：虚拟的「待办」，或某个真实项目。
///
/// 序列化成 `{ kind: "todo" }` / `{ kind: "project", id }`，前端原样回传。
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Container {
    Todo,
    Project { id: i64 },
}

impl Container {
    /// 对应的 `task.project_id`（NULL 即「待办」）。
    fn project_id(self) -> Option<i64> {
        match self {
            Container::Todo => None,
            Container::Project { id } => Some(id),
        }
    }
}

/// 左栏的一项：项目，或虚拟的「待办」（`id` 为 `None`、`is_todo` 为 `true`）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: Option<i64>,
    pub name: String,
    pub is_todo: bool,
    /// 顶层任务数。子任务不计数——左栏是导航，不是进度总表。
    pub task_count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Submodule {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Module {
    pub id: i64,
    pub name: String,
    pub submodules: Vec<Submodule>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: i64,
    pub project_id: Option<i64>,
    pub module_id: Option<i64>,
    pub submodule_id: Option<i64>,
    pub parent_task_id: Option<i64>,
    pub title: String,
    pub status: Status,
    pub priority: Priority,
    pub note: String,
    pub sort_order: i64,
}

// ---------------------------------------------------------------------------
// 读
// ---------------------------------------------------------------------------

/// 左栏全部条目：首项恒为「待办」，其余按 `sort_order`。
pub fn list_projects(conn: &Connection) -> Res<Vec<Project>> {
    let todo_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM task WHERE project_id IS NULL AND parent_task_id IS NULL",
            [],
            |row| row.get(0),
        )
        .map_err(db_err)?;

    let mut out = vec![Project {
        id: None,
        name: TODO_NAME.to_string(),
        is_todo: true,
        task_count: todo_count,
    }];

    let mut stmt = conn
        .prepare(
            "SELECT p.id, p.name,
                    (SELECT COUNT(*) FROM task t
                      WHERE t.project_id = p.id AND t.parent_task_id IS NULL)
               FROM project p
              ORDER BY p.sort_order, p.id",
        )
        .map_err(db_err)?;

    let rows = stmt
        .query_map([], |row| {
            Ok(Project {
                id: Some(row.get(0)?),
                name: row.get(1)?,
                is_todo: false,
                task_count: row.get(2)?,
            })
        })
        .map_err(db_err)?;

    for row in rows {
        out.push(row.map_err(db_err)?);
    }
    Ok(out)
}

/// 项目的模块树（模块 + 其子模块）。"待办"没有模块，不调用这个命令。
pub fn list_modules(conn: &Connection, project_id: i64) -> Res<Vec<Module>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name FROM module WHERE project_id = ?1 ORDER BY sort_order, id",
        )
        .map_err(db_err)?;
    let modules = stmt
        .query_map(params![project_id], |row| {
            Ok(Module {
                id: row.get(0)?,
                name: row.get(1)?,
                submodules: Vec::new(),
            })
        })
        .map_err(db_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db_err)?;
    drop(stmt);

    let mut sub_stmt = conn
        .prepare("SELECT id, name FROM submodule WHERE module_id = ?1 ORDER BY sort_order, id")
        .map_err(db_err)?;

    let mut out = Vec::with_capacity(modules.len());
    for mut module in modules {
        module.submodules = sub_stmt
            .query_map(params![module.id], |row| {
                Ok(Submodule {
                    id: row.get(0)?,
                    name: row.get(1)?,
                })
            })
            .map_err(db_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_err)?;
        out.push(module);
    }
    Ok(out)
}

/// 某个容器里的全部任务（含子任务；分组与排序在前端做）。
pub fn list_tasks(conn: &Connection, container: Container) -> Res<Vec<Task>> {
    let sql = "SELECT id, project_id, module_id, submodule_id, parent_task_id,
                      title, status, priority, note, sort_order
                 FROM task
                WHERE project_id IS ?1
                ORDER BY sort_order, id";
    let mut stmt = conn.prepare(sql).map_err(db_err)?;
    let rows = stmt
        .query_map(params![container.project_id()], map_task)
        .map_err(db_err)?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(db_err)?.into_task()?);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// 写
// ---------------------------------------------------------------------------

pub fn create_project(conn: &Connection, name: &str) -> Res<Project> {
    let name = require_name(name, "项目名")?;
    let sort_order = next_sort_order(conn, "project", "1 = 1", params![])?;

    conn.execute(
        "INSERT INTO project (name, sort_order) VALUES (?1, ?2)",
        params![name, sort_order],
    )
    .map_err(db_err)?;

    Ok(Project {
        id: Some(conn.last_insert_rowid()),
        name: name.to_string(),
        is_todo: false,
        task_count: 0,
    })
}

pub fn create_module(conn: &Connection, project_id: i64, name: &str) -> Res<Module> {
    let name = require_name(name, "模块名")?;
    require_project(conn, project_id)?;
    let sort_order = next_sort_order(
        conn,
        "module",
        "project_id = ?1",
        params![project_id],
    )?;

    conn.execute(
        "INSERT INTO module (project_id, name, sort_order) VALUES (?1, ?2, ?3)",
        params![project_id, name, sort_order],
    )
    .map_err(db_err)?;

    Ok(Module {
        id: conn.last_insert_rowid(),
        name: name.to_string(),
        submodules: Vec::new(),
    })
}

/// 新建顶层任务。
///
/// 归属规则：容器给项目，分类一律落"未分类"（`module_id`/`submodule_id` 为 NULL），
/// 优先级默认"中"，排序号取同组最大值 + 1（即排在组内最后）。
pub fn create_task(conn: &Connection, container: Container, title: &str) -> Res<Task> {
    let title = require_name(title, "任务文字")?;

    let project_id = container.project_id();
    if let Some(id) = project_id {
        require_project(conn, id)?;
    }

    let sort_order = next_sort_order(
        conn,
        "task",
        "project_id IS ?1 AND module_id IS NULL AND submodule_id IS NULL \
         AND parent_task_id IS NULL",
        params![project_id],
    )?;

    conn.execute(
        "INSERT INTO task (project_id, title, status, priority, sort_order) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            project_id,
            title,
            // 默认值由领域层写出来，不靠 schema 的 DEFAULT：规则只有一处真相。
            Status::Todo.as_str(),
            Priority::Mid.as_str(),
            sort_order
        ],
    )
    .map_err(db_err)?;

    Ok(Task {
        id: conn.last_insert_rowid(),
        project_id,
        module_id: None,
        submodule_id: None,
        parent_task_id: None,
        title: title.to_string(),
        status: Status::Todo,
        priority: Priority::Mid,
        note: String::new(),
        sort_order,
    })
}

// ---------------------------------------------------------------------------
// 内部
// ---------------------------------------------------------------------------

/// 行 → 任务。`status`/`priority` 是文本列，解析失败要在这一层炸出来。
struct TaskRow {
    id: i64,
    project_id: Option<i64>,
    module_id: Option<i64>,
    submodule_id: Option<i64>,
    parent_task_id: Option<i64>,
    title: String,
    status: String,
    priority: String,
    note: String,
    sort_order: i64,
}

impl TaskRow {
    fn into_task(self) -> Res<Task> {
        Ok(Task {
            id: self.id,
            project_id: self.project_id,
            module_id: self.module_id,
            submodule_id: self.submodule_id,
            parent_task_id: self.parent_task_id,
            title: self.title,
            status: Status::from_db(&self.status)?,
            priority: Priority::from_db(&self.priority)?,
            note: self.note,
            sort_order: self.sort_order,
        })
    }
}

fn map_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRow> {
    Ok(TaskRow {
        id: row.get(0)?,
        project_id: row.get(1)?,
        module_id: row.get(2)?,
        submodule_id: row.get(3)?,
        parent_task_id: row.get(4)?,
        title: row.get(5)?,
        status: row.get(6)?,
        priority: row.get(7)?,
        note: row.get(8)?,
        sort_order: row.get(9)?,
    })
}

/// 名字类字段统一 trim；trim 后为空则拒绝（前端也会拦，但后端不能只信任前端）。
fn require_name<'a>(raw: &'a str, field: &str) -> Res<&'a str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("{field}不能为空"));
    }
    Ok(trimmed)
}

fn require_project(conn: &Connection, id: i64) -> Res<()> {
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM project WHERE id = ?1)",
            params![id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if exists {
        Ok(())
    } else {
        Err(format!("项目 {id} 不存在"))
    }
}

/// 同组内下一个排序号（组内没有任务时为 0）。
///
/// `where_clause` 只能来自本文件里的字面量——它是 SQL 片段，不接受外部输入。
fn next_sort_order(
    conn: &Connection,
    table: &str,
    where_clause: &str,
    args: impl rusqlite::Params,
) -> Res<i64> {
    let sql = format!(
        "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM {table} WHERE {where_clause}"
    );
    conn.query_row(&sql, args, |row| row.get(0)).map_err(db_err)
}

fn db_err(err: rusqlite::Error) -> String {
    format!("数据库操作失败：{err}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 内存库 + 跑完迁移：领域层测试不需要窗口，也不需要真文件。
    fn conn() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        crate::migrations::migrate(&mut conn).unwrap();
        conn
    }

    /// 最小纵向切片的主链路：建项目 → 建任务 → 取回来。
    #[test]
    fn new_task_lands_unclassified_in_project() {
        let conn = conn();
        let project = create_project(&conn, "todolist-tr").unwrap();
        let project_id = project.id.unwrap();

        let task = create_task(
            &conn,
            Container::Project { id: project_id },
            "处理并发刷新",
        )
        .unwrap();

        assert_eq!(task.project_id, Some(project_id));
        assert_eq!(task.module_id, None, "新任务默认落在未分类");
        assert_eq!(task.submodule_id, None);
        assert_eq!(task.priority.as_str(), "mid", "默认优先级为中");
        assert_eq!(task.status.as_str(), "todo");
        assert_eq!(task.sort_order, 0);

        let tasks = list_tasks(&conn, Container::Project { id: project_id }).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "处理并发刷新");
    }

    /// 「待办」不是一行项目：它由 `project_id IS NULL` 表达，且不混进项目列表。
    #[test]
    fn todo_tasks_stay_out_of_projects() {
        let conn = conn();
        let project = create_project(&conn, "支付网关").unwrap();
        create_task(&conn, Container::Todo, "看一下 CI 偶发失败").unwrap();

        let projects = list_projects(&conn).unwrap();
        assert!(projects[0].is_todo, "首项恒为「待办」");
        assert_eq!(projects[0].id, None);
        assert_eq!(projects[0].task_count, 1);

        let real = projects.iter().find(|p| p.id == project.id).unwrap();
        assert_eq!(real.task_count, 0, "待办里的任务不算进项目");

        let tasks = list_tasks(&conn, Container::Todo).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].project_id, None);
    }

    /// 组内排序号递增：新任务排在组内最后。
    #[test]
    fn sort_order_appends_within_group() {
        let conn = conn();
        let project = create_project(&conn, "文档站").unwrap();
        let container = Container::Project { id: project.id.unwrap() };

        assert_eq!(create_task(&conn, container, "a").unwrap().sort_order, 0);
        assert_eq!(create_task(&conn, container, "b").unwrap().sort_order, 1);
        // 不同容器各排各的
        assert_eq!(create_task(&conn, Container::Todo, "c").unwrap().sort_order, 0);
    }

    /// 空白文字在前端会被拦，后端也必须自己拦——不能只信任前端。
    #[test]
    fn rejects_blank_names() {
        let conn = conn();
        assert!(create_project(&conn, "   ").is_err());
        assert!(create_task(&conn, Container::Todo, "\t\n").is_err());
    }

    /// 名字两侧空白被 trim，落到库里的是干净值。
    #[test]
    fn trims_names() {
        let conn = conn();
        let project = create_project(&conn, "  认证  ").unwrap();
        assert_eq!(project.name, "认证");
    }

    /// 往不存在的项目里建任务要报错，而不是留下一条悬空外键。
    #[test]
    fn rejects_unknown_project() {
        let conn = conn();
        assert!(create_task(&conn, Container::Project { id: 999 }, "x").is_err());
        assert!(create_module(&conn, 999, "m").is_err());
    }

    /// 模块树按 sort_order 组装，子模块挂在各自模块下。
    #[test]
    fn modules_carry_their_submodules() {
        let conn = conn();
        let project = create_project(&conn, "todolist-tr").unwrap();
        let project_id = project.id.unwrap();
        let auth = create_module(&conn, project_id, "认证").unwrap();
        create_module(&conn, project_id, "计费").unwrap();

        conn.execute(
            "INSERT INTO submodule (module_id, name, sort_order) VALUES (?1, ?2, ?3)",
            params![auth.id, "token 刷新", 0],
        )
        .unwrap();

        let modules = list_modules(&conn, project_id).unwrap();
        assert_eq!(
            modules.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
            vec!["认证", "计费"]
        );
        assert_eq!(modules[0].submodules.len(), 1);
        assert_eq!(modules[0].submodules[0].name, "token 刷新");
        assert!(modules[1].submodules.is_empty());
    }
}
