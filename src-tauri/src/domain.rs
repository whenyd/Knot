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

/// 能整组删除的两级分类节点。
///
/// 模块和子模块的 id 各自独立（都从 1 开始），只传一个 id 无法区分，
/// 所以命令必须同时带上 kind。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Module,
    Submodule,
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

#[derive(Debug, Serialize, Clone)]
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

/// `plan_set_status` 的返回：改这个状态之前要告诉前端什么。
///
/// 两段式的意义全在 `needs_confirm` 上：前端不重算规则，只根据它决定要不要弹窗。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusPlan {
    /// 会受影响的任务（目标任务 + 需要一并改的子任务）。用来告知条数。
    pub affects: Vec<Task>,
    pub needs_confirm: bool,
    /// 需要确认时给用户看的一句话；不需要确认时为空串。
    pub reason: String,
}

/// `apply_set_status` 的返回。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusOutcome {
    pub changed: Vec<Task>,
    /// 父任务被自动改动时的轻提示。用户自己点的这一下不产生提示。
    pub notices: Vec<String>,
}

/// 删除前的告知：这次会删掉多少个任务（含子任务）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletePlan {
    pub task_count: i64,
}

/// 任务表的列顺序，读任务的地方只有这一份，避免 `row.get(i)` 的序号漂移。
const TASK_COLUMNS: &str = "id, project_id, module_id, submodule_id, parent_task_id, \
                            title, status, priority, note, sort_order";

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
    let sql = format!(
        "SELECT {TASK_COLUMNS} FROM task WHERE project_id IS ?1 ORDER BY sort_order, id"
    );
    let mut stmt = conn.prepare(&sql).map_err(db_err)?;
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

/// 新建子模块。最多两级分类，所以子模块下面不能再挂东西。
pub fn create_submodule(conn: &Connection, module_id: i64, name: &str) -> Res<Submodule> {
    let name = require_name(name, "子模块名")?;
    require_module_exists(conn, module_id)?;
    let sort_order = next_sort_order(conn, "submodule", "module_id = ?1", params![module_id])?;

    conn.execute(
        "INSERT INTO submodule (module_id, name, sort_order) VALUES (?1, ?2, ?3)",
        params![module_id, name, sort_order],
    )
    .map_err(db_err)?;

    Ok(Submodule {
        id: conn.last_insert_rowid(),
        name: name.to_string(),
    })
}

pub fn rename_project(conn: &Connection, id: i64, name: &str) -> Res<Project> {
    rename_row(conn, "project", id, name, "项目", "项目名")?;
    load_project(conn, id)
}

pub fn rename_module(conn: &Connection, id: i64, name: &str) -> Res<Module> {
    rename_row(conn, "module", id, name, "模块", "模块名")?;
    load_module(conn, id)
}

pub fn rename_submodule(conn: &Connection, id: i64, name: &str) -> Res<Submodule> {
    rename_row(conn, "submodule", id, name, "子模块", "子模块名")?;
    load_submodule(conn, id)
}

/// 新建顶层任务。
///
/// 归属规则：容器给项目，`module_id`/`submodule_id` 由调用方按"当前范围"给出，
/// 不给就是"未分类"。优先级默认"中"，排序号取**同一组内**最大值 + 1（排在组内最后）。
pub fn create_task(
    conn: &Connection,
    container: Container,
    module_id: Option<i64>,
    submodule_id: Option<i64>,
    title: &str,
) -> Res<Task> {
    let title = require_name(title, "任务文字")?;

    let project_id = container.project_id();
    match (project_id, module_id) {
        (Some(project_id), Some(module_id)) => {
            require_project(conn, project_id)?;
            require_module(conn, project_id, module_id)?;
        }
        // 「待办」没有模块，带模块归属是调用方的错
        (None, Some(_)) => return Err("「待办」里的任务不属于任何模块".to_string()),
        (Some(project_id), None) => require_project(conn, project_id)?,
        (None, None) => {}
    }
    if let Some(submodule_id) = submodule_id {
        let module_id = module_id.ok_or("子模块必须附属于一个模块")?;
        require_submodule(conn, module_id, submodule_id)?;
    }

    let sort_order = next_sort_order(
        conn,
        "task",
        "project_id IS ?1 AND module_id IS ?2 AND submodule_id IS ?3 \
         AND parent_task_id IS NULL",
        params![project_id, module_id, submodule_id],
    )?;

    conn.execute(
        "INSERT INTO task (project_id, module_id, submodule_id, title, status, priority, sort_order) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            project_id,
            module_id,
            submodule_id,
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
        module_id,
        submodule_id,
        parent_task_id: None,
        title: title.to_string(),
        status: Status::Todo,
        priority: Priority::Mid,
        note: String::new(),
        sort_order,
    })
}

/// 就地改名。trim 后为空就拒绝——前端会先把原文还原，库里一个字节也不动。
pub fn rename_task(conn: &Connection, id: i64, title: &str) -> Res<Task> {
    let title = require_name(title, "任务文字")?;
    let mut task = load_task(conn, id)?;

    conn.execute(
        "UPDATE task SET title = ?1 WHERE id = ?2",
        params![title, id],
    )
    .map_err(db_err)?;

    task.title = title.to_string();
    Ok(task)
}

/// 改优先级。不涉及确认：组内重排是这个字段的代价，不需要问。
pub fn set_priority(conn: &Connection, id: i64, priority: Priority) -> Res<Task> {
    let mut task = load_task(conn, id)?;

    conn.execute(
        "UPDATE task SET priority = ?1 WHERE id = ?2",
        params![priority.as_str(), id],
    )
    .map_err(db_err)?;

    task.priority = priority;
    Ok(task)
}

/// 改状态的第一步：只算“要不要确认”，不写库。
///
/// 需要确认只有一个场景：把还有未完成子任务的任务标为已完成。
/// 退回（改成未完成/进行中）永远不联动子任务，所以不需要确认。
pub fn plan_set_status(conn: &Connection, id: i64, status: Status) -> Res<StatusPlan> {
    let task = load_task(conn, id)?;
    let open_children = if status == Status::Done {
        unfinished_children(conn, id)?
    } else {
        Vec::new()
    };

    let needs_confirm = !open_children.is_empty();
    let reason = if needs_confirm {
        format!(
            "这个任务还有 {} 个未完成子任务，确认后会一并标记为已完成。",
            open_children.len()
        )
    } else {
        String::new()
    };

    let mut affects = vec![task];
    affects.extend(open_children);

    Ok(StatusPlan {
        affects,
        needs_confirm,
        reason,
    })
}

/// 改状态的第二步：写库。`cascade` 为真时连同未完成子任务一起标为已完成。
///
/// 父任务被“聚合规则”自动推导（子任务全完成 → 父完成）不在本命令里，
/// 那是子任务状态变化时的事，走另一条路径。
pub fn apply_set_status(
    conn: &Connection,
    id: i64,
    status: Status,
    cascade: bool,
) -> Res<StatusOutcome> {
    let mut task = load_task(conn, id)?;
    let tx = conn.unchecked_transaction().map_err(db_err)?;

    write_status(&tx, &mut task, status)?;
    let mut changed = vec![task];

    if cascade && status == Status::Done {
        for mut child in unfinished_children(&tx, id)? {
            write_status(&tx, &mut child, Status::Done)?;
            changed.push(child);
        }
    }

    tx.commit().map_err(db_err)?;

    Ok(StatusOutcome {
        changed,
        // “父任务被自动改动才提示”还没到：本切片还没有能把父任务带动的路径。
        notices: Vec::new(),
    })
}

// ---------------------------------------------------------------------------
// 删除
// ---------------------------------------------------------------------------

/// 删除任务的第一步：只数会删掉几条，不写库。
///
/// 子任务靠 `parent_task_id` 级联删除，所以这里的条数必须把它们算进去；
/// 前端拿这个数字直接告诉用户。
pub fn plan_delete_task(conn: &Connection, id: i64) -> Res<DeletePlan> {
    load_task(conn, id)?;
    let task_count: i64 = conn
        .query_row(
            "WITH RECURSIVE doomed(id) AS ( \
                 SELECT ?1 \
                 UNION \
                 SELECT t.id FROM task t JOIN doomed d ON t.parent_task_id = d.id \
             ) SELECT COUNT(*) FROM doomed",
            params![id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    Ok(DeletePlan { task_count })
}

pub fn delete_task(conn: &Connection, id: i64) -> Res<()> {
    let removed = conn
        .execute("DELETE FROM task WHERE id = ?1", params![id])
        .map_err(db_err)?;
    if removed == 0 {
        return Err(format!("任务 {id} 不存在"));
    }
    Ok(())
}

/// 删除模块 / 子模块的第一步：只数会删掉几条，不写库。
///
/// 模块的条数要把其子模块里的任务也算进来——它们会一起级联删除。
pub fn plan_delete_node(conn: &Connection, kind: NodeKind, id: i64) -> Res<DeletePlan> {
    let task_count: i64 = match kind {
        NodeKind::Module => {
            require_module_exists(conn, id)?;
            conn.query_row(
                "WITH RECURSIVE doomed(id) AS ( \
                     SELECT id FROM task \
                      WHERE module_id = ?1 \
                         OR submodule_id IN (SELECT id FROM submodule WHERE module_id = ?1) \
                     UNION \
                     SELECT t.id FROM task t JOIN doomed d ON t.parent_task_id = d.id \
                 ) SELECT COUNT(*) FROM doomed",
                params![id],
                |row| row.get(0),
            )
            .map_err(db_err)?
        }
        NodeKind::Submodule => {
            require_submodule_exists(conn, id)?;
            conn.query_row(
                "WITH RECURSIVE doomed(id) AS ( \
                     SELECT id FROM task WHERE submodule_id = ?1 \
                     UNION \
                     SELECT t.id FROM task t JOIN doomed d ON t.parent_task_id = d.id \
                 ) SELECT COUNT(*) FROM doomed",
                params![id],
                |row| row.get(0),
            )
            .map_err(db_err)?
        }
    };
    Ok(DeletePlan { task_count })
}

pub fn delete_node(conn: &Connection, kind: NodeKind, id: i64) -> Res<()> {
    let (table, label) = match kind {
        NodeKind::Module => ("module", "模块"),
        NodeKind::Submodule => ("submodule", "子模块"),
    };
    let removed = conn
        .execute(&format!("DELETE FROM {table} WHERE id = ?1"), params![id])
        .map_err(db_err)?;
    if removed == 0 {
        return Err(format!("{label} {id} 不存在"));
    }
    Ok(())
}

/// 删除项目的第一步：项目里的全部任务都会随项目级联删除。
pub fn plan_delete_project(conn: &Connection, id: i64) -> Res<DeletePlan> {
    require_project(conn, id)?;
    let task_count: i64 = conn
        .query_row(
            "WITH RECURSIVE doomed(id) AS ( \
                 SELECT id FROM task WHERE project_id = ?1 \
                 UNION \
                 SELECT t.id FROM task t JOIN doomed d ON t.parent_task_id = d.id \
             ) SELECT COUNT(*) FROM doomed",
            params![id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    Ok(DeletePlan { task_count })
}

pub fn delete_project(conn: &Connection, id: i64) -> Res<()> {
    let removed = conn
        .execute("DELETE FROM project WHERE id = ?1", params![id])
        .map_err(db_err)?;
    if removed == 0 {
        return Err(format!("项目 {id} 不存在"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 内部
// ---------------------------------------------------------------------------

fn write_status(conn: &Connection, task: &mut Task, status: Status) -> Res<()> {
    conn.execute(
        "UPDATE task SET status = ?1 WHERE id = ?2",
        params![status.as_str(), task.id],
    )
    .map_err(db_err)?;
    task.status = status;
    Ok(())
}

fn load_task(conn: &Connection, id: i64) -> Res<Task> {
    let sql = format!("SELECT {TASK_COLUMNS} FROM task WHERE id = ?1");
    conn.query_row(&sql, params![id], map_task)
        .map_err(|err| match err {
            rusqlite::Error::QueryReturnedNoRows => format!("任务 {id} 不存在"),
            other => db_err(other),
        })?
        .into_task()
}

/// 某个任务下还没完成的子任务（一层，不递归：子任务不能再挂子任务）。
fn unfinished_children(conn: &Connection, parent_id: i64) -> Res<Vec<Task>> {
    let sql = format!(
        "SELECT {TASK_COLUMNS} FROM task \
          WHERE parent_task_id = ?1 AND status != 'done' ORDER BY sort_order, id"
    );
    let mut stmt = conn.prepare(&sql).map_err(db_err)?;
    let rows = stmt
        .query_map(params![parent_id], map_task)
        .map_err(db_err)?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(db_err)?.into_task()?);
    }
    Ok(out)
}
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

fn require_module_exists(conn: &Connection, module_id: i64) -> Res<()> {
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM module WHERE id = ?1)",
            params![module_id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if exists {
        Ok(())
    } else {
        Err(format!("模块 {module_id} 不存在"))
    }
}

fn require_submodule_exists(conn: &Connection, submodule_id: i64) -> Res<()> {
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM submodule WHERE id = ?1)",
            params![submodule_id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if exists {
        Ok(())
    } else {
        Err(format!("子模块 {submodule_id} 不存在"))
    }
}

/// 改一个只有 `name` 的结构行。`table`/`label`/`field` 只接受本文件里的字面量。
fn rename_row(
    conn: &Connection,
    table: &str,
    id: i64,
    name: &str,
    label: &str,
    field: &str,
) -> Res<()> {
    let name = require_name(name, field)?;
    let changed = conn
        .execute(
            &format!("UPDATE {table} SET name = ?1 WHERE id = ?2"),
            params![name, id],
        )
        .map_err(db_err)?;

    if changed == 0 {
        return Err(format!("{label} {id} 不存在"));
    }
    Ok(())
}

fn load_project(conn: &Connection, id: i64) -> Res<Project> {
    conn.query_row(
        "SELECT p.id, p.name,
                (SELECT COUNT(*) FROM task t
                  WHERE t.project_id = p.id AND t.parent_task_id IS NULL)
           FROM project p WHERE p.id = ?1",
        params![id],
        |row| {
            Ok(Project {
                id: Some(row.get(0)?),
                name: row.get(1)?,
                is_todo: false,
                task_count: row.get(2)?,
            })
        },
    )
    .map_err(|err| match err {
        rusqlite::Error::QueryReturnedNoRows => format!("项目 {id} 不存在"),
        other => db_err(other),
    })
}

fn load_module(conn: &Connection, id: i64) -> Res<Module> {
    let mut module = conn
        .query_row(
            "SELECT id, name FROM module WHERE id = ?1",
            params![id],
            |row| {
                Ok(Module {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    submodules: Vec::new(),
                })
            },
        )
        .map_err(|err| match err {
            rusqlite::Error::QueryReturnedNoRows => format!("模块 {id} 不存在"),
            other => db_err(other),
        })?;

    let mut stmt = conn
        .prepare("SELECT id, name FROM submodule WHERE module_id = ?1 ORDER BY sort_order, id")
        .map_err(db_err)?;
    module.submodules = stmt
        .query_map(params![id], |row| {
            Ok(Submodule {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        })
        .map_err(db_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db_err)?;

    Ok(module)
}

fn load_submodule(conn: &Connection, id: i64) -> Res<Submodule> {
    conn.query_row(
        "SELECT id, name FROM submodule WHERE id = ?1",
        params![id],
        |row| {
            Ok(Submodule {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        },
    )
    .map_err(|err| match err {
        rusqlite::Error::QueryReturnedNoRows => format!("子模块 {id} 不存在"),
        other => db_err(other),
    })
}

/// 模块必须真的属于这个项目：不能把 A 项目的模块挂到 B 项目的任务上。
fn require_module(conn: &Connection, project_id: i64, module_id: i64) -> Res<()> {
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM module WHERE id = ?1 AND project_id = ?2)",
            params![module_id, project_id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if exists {
        Ok(())
    } else {
        Err(format!("模块 {module_id} 不属于项目 {project_id}"))
    }
}

/// 子模块必须真的属于这个模块。
fn require_submodule(conn: &Connection, module_id: i64, submodule_id: i64) -> Res<()> {
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM submodule WHERE id = ?1 AND module_id = ?2)",
            params![submodule_id, module_id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if exists {
        Ok(())
    } else {
        Err(format!("子模块 {submodule_id} 不属于模块 {module_id}"))
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
            None,
            None,
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
        create_task(&conn, Container::Todo, None, None, "看一下 CI 偶发失败").unwrap();

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

        assert_eq!(
            create_task(&conn, container, None, None, "a").unwrap().sort_order,
            0
        );
        assert_eq!(
            create_task(&conn, container, None, None, "b").unwrap().sort_order,
            1
        );
        // 不同容器各排各的
        assert_eq!(
            create_task(&conn, Container::Todo, None, None, "c")
                .unwrap()
                .sort_order,
            0
        );
    }

    /// 空白文字在前端会被拦，后端也必须自己拦——不能只信任前端。
    #[test]
    fn rejects_blank_names() {
        let conn = conn();
        assert!(create_project(&conn, "   ").is_err());
        assert!(create_task(&conn, Container::Todo, None, None, "\t\n").is_err());
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
        assert!(create_task(&conn, Container::Project { id: 999 }, None, None, "x").is_err());
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

    // -----------------------------------------------------------------------
    // 新任务落入当前范围
    // -----------------------------------------------------------------------

    fn seed_submodule(conn: &Connection, module_id: i64, name: &str) -> i64 {
        conn.execute(
            "INSERT INTO submodule (module_id, name, sort_order) VALUES (?1, ?2, 0)",
            params![module_id, name],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn task_created_in_module_lands_there() {
        let conn = conn();
        let project = create_project(&conn, "todolist-tr").unwrap();
        let project_id = project.id.unwrap();
        let module = create_module(&conn, project_id, "认证").unwrap();
        seed_submodule(&conn, module.id, "token 刷新");

        let task = create_task(
            &conn,
            Container::Project { id: project_id },
            Some(module.id),
            None,
            "登录限流",
        )
        .unwrap();

        assert_eq!(task.module_id, Some(module.id));
        assert_eq!(task.submodule_id, None, "只选模块时落在模块的直接列表");
    }

    #[test]
    fn task_created_in_submodule_lands_there() {
        let conn = conn();
        let project = create_project(&conn, "todolist-tr").unwrap();
        let project_id = project.id.unwrap();
        let module = create_module(&conn, project_id, "认证").unwrap();
        let submodule = seed_submodule(&conn, module.id, "token 刷新");

        let task = create_task(
            &conn,
            Container::Project { id: project_id },
            Some(module.id),
            Some(submodule),
            "补过期用例",
        )
        .unwrap();

        assert_eq!(task.module_id, Some(module.id));
        assert_eq!(task.submodule_id, Some(submodule));
    }

    /// 排序号按**同一组**算：模块里的任务与未分类的任务各排各的。
    #[test]
    fn sort_order_is_per_group() {
        let conn = conn();
        let project = create_project(&conn, "todolist-tr").unwrap();
        let project_id = project.id.unwrap();
        let container = Container::Project { id: project_id };
        let module = create_module(&conn, project_id, "认证").unwrap();

        assert_eq!(
            create_task(&conn, container, None, None, "未分类的")
                .unwrap()
                .sort_order,
            0
        );
        assert_eq!(
            create_task(&conn, container, Some(module.id), None, "模块里的")
                .unwrap()
                .sort_order,
            0,
            "另一个组从头数起"
        );
        assert_eq!(
            create_task(&conn, container, Some(module.id), None, "模块里的第二条")
                .unwrap()
                .sort_order,
            1
        );
    }

    /// 模块、子模块归属必须是真的：不能跨项目、不能张冠李戴。
    #[test]
    fn rejects_module_from_other_project() {
        let conn = conn();
        let a = create_project(&conn, "A").unwrap().id.unwrap();
        let b = create_project(&conn, "B").unwrap().id.unwrap();
        let module = create_module(&conn, a, "认证").unwrap();

        assert!(create_task(&conn, Container::Project { id: b }, Some(module.id), None, "x").is_err());
    }

    #[test]
    fn rejects_submodule_from_other_module() {
        let conn = conn();
        let project_id = create_project(&conn, "todolist-tr").unwrap().id.unwrap();
        let auth = create_module(&conn, project_id, "认证").unwrap();
        let billing = create_module(&conn, project_id, "计费").unwrap();
        let submodule = seed_submodule(&conn, auth.id, "token 刷新");

        assert!(create_task(
            &conn,
            Container::Project { id: project_id },
            Some(billing.id),
            Some(submodule),
            "x"
        )
        .is_err());
    }

    /// 「待办」没有模块，带模块归属是调用方的错，不是默默把它当未分类。
    #[test]
    fn rejects_module_on_todo_container() {
        let conn = conn();
        let project_id = create_project(&conn, "todolist-tr").unwrap().id.unwrap();
        let module = create_module(&conn, project_id, "认证").unwrap();

        assert!(create_task(&conn, Container::Todo, Some(module.id), None, "x").is_err());
    }

    // -----------------------------------------------------------------------
    // 结构：新建子模块与三级重命名
    // -----------------------------------------------------------------------

    #[test]
    fn create_submodule_appends_in_its_module() {
        let conn = conn();
        let project_id = create_project(&conn, "todolist-tr").unwrap().id.unwrap();
        let auth = create_module(&conn, project_id, "认证").unwrap();
        let billing = create_module(&conn, project_id, "计费").unwrap();

        let first = create_submodule(&conn, auth.id, "token 刷新").unwrap();
        let second = create_submodule(&conn, auth.id, "登录限流").unwrap();

        let modules = list_modules(&conn, project_id).unwrap();
        let auth_now = modules.iter().find(|m| m.id == auth.id).unwrap();
        assert_eq!(
            auth_now.submodules.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            vec!["token 刷新", "登录限流"]
        );
        assert_eq!(auth_now.submodules[0].id, first.id);
        assert_eq!(auth_now.submodules[1].id, second.id);

        let billing_now = modules.iter().find(|m| m.id == billing.id).unwrap();
        assert!(billing_now.submodules.is_empty(), "排序号按模块各算各的");
    }

    #[test]
    fn create_submodule_rejects_unknown_module() {
        let conn = conn();
        assert!(create_submodule(&conn, 999, "x").is_err());
    }

    #[test]
    fn create_submodule_rejects_blank_name() {
        let conn = conn();
        let project_id = create_project(&conn, "todolist-tr").unwrap().id.unwrap();
        let module = create_module(&conn, project_id, "认证").unwrap();
        assert!(create_submodule(&conn, module.id, "   ").is_err());
    }

    #[test]
    fn renames_all_three_levels() {
        let conn = conn();
        let project = create_project(&conn, "todolist-tr").unwrap();
        let project_id = project.id.unwrap();
        let module = create_module(&conn, project_id, "认证").unwrap();
        let submodule = create_submodule(&conn, module.id, "token 刷新").unwrap();

        let renamed_project = rename_project(&conn, project_id, "  todolist  ").unwrap();
        assert_eq!(renamed_project.name, "todolist");
        assert_eq!(renamed_project.id, Some(project_id), "返回值仍是同一个项目");

        let renamed_module = rename_module(&conn, module.id, "鉴权").unwrap();
        assert_eq!(renamed_module.name, "鉴权");
        assert_eq!(
            renamed_module.submodules.len(),
            1,
            "改模块名要把它现有的子模块一并返回"
        );

        let renamed_submodule = rename_submodule(&conn, submodule.id, "刷新令牌").unwrap();
        assert_eq!(renamed_submodule.name, "刷新令牌");

        // 落到库里也真的改了
        let modules = list_modules(&conn, project_id).unwrap();
        assert_eq!(modules[0].name, "鉴权");
        assert_eq!(modules[0].submodules[0].name, "刷新令牌");
        let projects = list_projects(&conn).unwrap();
        assert_eq!(projects[1].name, "todolist");
    }

    /// 空白名被拒，且库里的旧名字不受影响。
    #[test]
    fn rename_rejects_blank_for_every_level() {
        let conn = conn();
        let project_id = create_project(&conn, "todolist-tr").unwrap().id.unwrap();
        let module = create_module(&conn, project_id, "认证").unwrap();
        let submodule = create_submodule(&conn, module.id, "token 刷新").unwrap();

        assert!(rename_project(&conn, project_id, "  ").is_err());
        assert!(rename_module(&conn, module.id, "").is_err());
        assert!(rename_submodule(&conn, submodule.id, "\t").is_err());

        assert_eq!(load_project(&conn, project_id).unwrap().name, "todolist-tr");
        assert_eq!(load_module(&conn, module.id).unwrap().name, "认证");
        assert_eq!(load_submodule(&conn, submodule.id).unwrap().name, "token 刷新");
    }

    #[test]
    fn rename_unknown_rows_error() {
        let conn = conn();
        assert!(rename_project(&conn, 999, "x").is_err());
        assert!(rename_module(&conn, 999, "x").is_err());
        assert!(rename_submodule(&conn, 999, "x").is_err());
    }

    /// 改结构名不该动里面的任务。
    #[test]
    fn rename_does_not_touch_tasks() {
        let conn = conn();
        let project_id = create_project(&conn, "todolist-tr").unwrap().id.unwrap();
        let module = create_module(&conn, project_id, "认证").unwrap();
        let task = create_task(
            &conn,
            Container::Project { id: project_id },
            Some(module.id),
            None,
            "登录限流",
        )
        .unwrap();

        rename_project(&conn, project_id, "todolist").unwrap();
        rename_module(&conn, module.id, "鉴权").unwrap();

        let after = load_task(&conn, task.id).unwrap();
        assert_eq!(after.module_id, Some(module.id));
        assert_eq!(after.title, "登录限流");
    }

    /// 有子模块必项有模块，不能只给子模块。
    #[test]
    fn rejects_submodule_without_module() {
        let conn = conn();
        let project_id = create_project(&conn, "todolist-tr").unwrap().id.unwrap();
        let module = create_module(&conn, project_id, "认证").unwrap();
        let submodule = seed_submodule(&conn, module.id, "token 刷新");

        assert!(create_task(
            &conn,
            Container::Project { id: project_id },
            None,
            Some(submodule),
            "x"
        )
        .is_err());
    }

    // -----------------------------------------------------------------------
    // 可编辑三件套
    // -----------------------------------------------------------------------

    /// 直接往库里塞一条子任务——本切片还没有建子任务的命令，但规则已经在这里。
    fn insert_child(conn: &Connection, parent: &Task, title: &str, status: Status) -> i64 {
        conn.execute(
            "INSERT INTO task (project_id, parent_task_id, title, status, priority) \
             VALUES (?1, ?2, ?3, ?4, 'mid')",
            params![parent.project_id, parent.id, title, status.as_str()],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn seed_task(conn: &Connection) -> Task {
        let container = Container::Todo;
        create_task(conn, container, None, None, "实现 token 轮换").unwrap()
    }

    #[test]
    fn rename_trims_and_persists() {
        let conn = conn();
        let task = seed_task(&conn);

        let renamed = rename_task(&conn, task.id, "  处理并发刷新  ").unwrap();
        assert_eq!(renamed.title, "处理并发刷新");
        assert_eq!(load_task(&conn, task.id).unwrap().title, "处理并发刷新");
    }

    /// 空白名字被拒，库里保留原文——前端还原原文时不会出现“已经清空了”的偏差。
    #[test]
    fn rename_rejects_blank_and_keeps_original() {
        let conn = conn();
        let task = seed_task(&conn);

        assert!(rename_task(&conn, task.id, "   ").is_err());
        assert_eq!(load_task(&conn, task.id).unwrap().title, "实现 token 轮换");
    }

    #[test]
    fn rename_unknown_task_errors() {
        let conn = conn();
        assert!(rename_task(&conn, 999, "x").is_err());
    }

    #[test]
    fn priority_is_persisted() {
        let conn = conn();
        let task = seed_task(&conn);

        let updated = set_priority(&conn, task.id, Priority::High).unwrap();
        assert_eq!(updated.priority.as_str(), "high");
        assert_eq!(load_task(&conn, task.id).unwrap().priority.as_str(), "high");
    }

    /// 没有子任务就没什么要确认的。
    #[test]
    fn setting_done_without_children_needs_no_confirm() {
        let conn = conn();
        let task = seed_task(&conn);

        let plan = plan_set_status(&conn, task.id, Status::Done).unwrap();
        assert!(!plan.needs_confirm);
        assert!(plan.reason.is_empty());
        assert_eq!(plan.affects.len(), 1);
    }

    /// 先算后写：`plan` 不能碰库。
    #[test]
    fn plan_does_not_write() {
        let conn = conn();
        let task = seed_task(&conn);

        plan_set_status(&conn, task.id, Status::Done).unwrap();
        assert_eq!(load_task(&conn, task.id).unwrap().status.as_str(), "todo");
    }

    #[test]
    fn setting_done_with_open_children_needs_confirm() {
        let conn = conn();
        let task = seed_task(&conn);
        insert_child(&conn, &task, "处理并发刷新", Status::Doing);
        insert_child(&conn, &task, "补过期用例", Status::Done); // 已完成的不算

        let plan = plan_set_status(&conn, task.id, Status::Done).unwrap();
        assert!(plan.needs_confirm);
        assert_eq!(plan.affects.len(), 2, "只把未完成的那个算进去");
        assert!(plan.reason.contains('1'));
    }

    /// 退回不需要确认：子任务不动。
    #[test]
    fn setting_back_needs_no_confirm() {
        let conn = conn();
        let task = seed_task(&conn);
        insert_child(&conn, &task, "处理并发刷新", Status::Todo);
        apply_set_status(&conn, task.id, Status::Done, true).unwrap();

        let plan = plan_set_status(&conn, task.id, Status::Doing).unwrap();
        assert!(!plan.needs_confirm);
    }

    #[test]
    fn cascade_done_marks_open_children() {
        let conn = conn();
        let task = seed_task(&conn);
        let open = insert_child(&conn, &task, "处理并发刷新", Status::Doing);
        let closed = insert_child(&conn, &task, "补过期用例", Status::Done);

        let outcome = apply_set_status(&conn, task.id, Status::Done, true).unwrap();
        assert_eq!(outcome.changed.len(), 2, "父任务 + 一个未完成子任务");
        assert!(outcome.notices.is_empty(), "用户自己点的这一下不产生提示");

        assert_eq!(load_task(&conn, task.id).unwrap().status.as_str(), "done");
        assert_eq!(load_task(&conn, open).unwrap().status.as_str(), "done");
        assert_eq!(load_task(&conn, closed).unwrap().status.as_str(), "done");
    }

    /// 用户选“不级联”时，子任务一条都不动。
    #[test]
    fn no_cascade_leaves_children_alone() {
        let conn = conn();
        let task = seed_task(&conn);
        let open = insert_child(&conn, &task, "处理并发刷新", Status::Todo);

        let outcome = apply_set_status(&conn, task.id, Status::Done, false).unwrap();
        assert_eq!(outcome.changed.len(), 1);
        assert_eq!(load_task(&conn, open).unwrap().status.as_str(), "todo");
    }

    /// 退回父任务时子任务保持不动（可能暂时不一致，这是有意的）。
    #[test]
    fn setting_back_leaves_children_alone() {
        let conn = conn();
        let task = seed_task(&conn);
        let open = insert_child(&conn, &task, "处理并发刷新", Status::Todo);

        apply_set_status(&conn, task.id, Status::Done, true).unwrap();
        apply_set_status(&conn, task.id, Status::Todo, true).unwrap();

        assert_eq!(load_task(&conn, task.id).unwrap().status.as_str(), "todo");
        assert_eq!(load_task(&conn, open).unwrap().status.as_str(), "done");
    }

    #[test]
    fn status_of_unknown_task_errors() {
        let conn = conn();
        assert!(plan_set_status(&conn, 999, Status::Done).is_err());
        assert!(apply_set_status(&conn, 999, Status::Done, false).is_err());
    }

    // -----------------------------------------------------------------------
    // 删除
    // -----------------------------------------------------------------------

    /// 先数后删：条数包含子任务，plan 本身不动库。
    #[test]
    fn delete_task_counts_children_then_cascades() {
        let conn = conn();
        let task = seed_task(&conn);
        let open = insert_child(&conn, &task, "处理并发刷新", Status::Todo);
        let done = insert_child(&conn, &task, "补过期用例", Status::Done);

        let plan = plan_delete_task(&conn, task.id).unwrap();
        assert_eq!(plan.task_count, 3, "父任务 + 两个子任务");
        assert_eq!(load_task(&conn, task.id).unwrap().title, "实现 token 轮换");

        delete_task(&conn, task.id).unwrap();
        assert!(load_task(&conn, task.id).is_err());
        assert!(load_task(&conn, open).is_err());
        assert!(load_task(&conn, done).is_err());
    }

    /// 删模块：子模块、模块里的任务、子模块里的任务及其子任务一起消失，
    /// 未分类任务与别的项目不受影响。
    #[test]
    fn delete_module_counts_submodule_tasks_and_cascades() {
        let conn = conn();
        let project_id = create_project(&conn, "todolist-tr").unwrap().id.unwrap();
        let container = Container::Project { id: project_id };
        let module = create_module(&conn, project_id, "认证").unwrap();
        let submodule = create_submodule(&conn, module.id, "token 刷新").unwrap();

        let in_module = create_task(&conn, container, Some(module.id), None, "登录限流").unwrap();
        let in_submodule = create_task(
            &conn,
            container,
            Some(module.id),
            Some(submodule.id),
            "补过期用例",
        )
        .unwrap();
        let child = insert_child(&conn, &in_submodule, "先写失败用例", Status::Todo);
        let outside = create_task(&conn, container, None, None, "整理文档").unwrap();

        let plan = plan_delete_node(&conn, NodeKind::Module, module.id).unwrap();
        assert_eq!(plan.task_count, 3, "模块任务 + 子模块任务 + 子任务");
        assert_eq!(list_tasks(&conn, container).unwrap().len(), 4, "plan 不写库");

        delete_node(&conn, NodeKind::Module, module.id).unwrap();

        let remaining = list_tasks(&conn, container).unwrap();
        assert_eq!(
            remaining.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
            vec!["整理文档"]
        );
        assert!(list_modules(&conn, project_id).unwrap().is_empty(), "子模块随模块删除");
        assert!(load_task(&conn, in_module.id).is_err());
        assert!(load_task(&conn, in_submodule.id).is_err());
        assert!(load_task(&conn, child).is_err());
        assert_eq!(load_task(&conn, outside.id).unwrap().title, "整理文档");
    }

    /// 删子模块只带走子模块里的任务，模块直属任务保留。
    #[test]
    fn delete_submodule_leaves_module_tasks_alone() {
        let conn = conn();
        let project_id = create_project(&conn, "todolist-tr").unwrap().id.unwrap();
        let container = Container::Project { id: project_id };
        let module = create_module(&conn, project_id, "认证").unwrap();
        let submodule = create_submodule(&conn, module.id, "token 刷新").unwrap();

        let in_module = create_task(&conn, container, Some(module.id), None, "登录限流").unwrap();
        let in_submodule = create_task(
            &conn,
            container,
            Some(module.id),
            Some(submodule.id),
            "补过期用例",
        )
        .unwrap();
        let child = insert_child(&conn, &in_submodule, "先写失败用例", Status::Todo);

        let plan = plan_delete_node(&conn, NodeKind::Submodule, submodule.id).unwrap();
        assert_eq!(plan.task_count, 2);

        delete_node(&conn, NodeKind::Submodule, submodule.id).unwrap();
        assert!(load_task(&conn, in_submodule.id).is_err());
        assert!(load_task(&conn, child).is_err());
        assert_eq!(load_task(&conn, in_module.id).unwrap().title, "登录限流");

        let modules = list_modules(&conn, project_id).unwrap();
        assert_eq!(modules.len(), 1);
        assert!(modules[0].submodules.is_empty());
    }

    /// 删项目：项目里的所有任务（含未分类、子任务）与模块树全部消失，
    /// 待办和别的项目一条都不动。
    #[test]
    fn delete_project_counts_all_tasks_and_cascades() {
        let conn = conn();
        let doomed = create_project(&conn, "todolist-tr").unwrap().id.unwrap();
        let kept = create_project(&conn, "支付网关").unwrap().id.unwrap();
        let container = Container::Project { id: doomed };

        let module = create_module(&conn, doomed, "认证").unwrap();
        let submodule = create_submodule(&conn, module.id, "token 刷新").unwrap();
        create_task(&conn, container, Some(module.id), None, "登录限流").unwrap();
        let in_submodule = create_task(
            &conn,
            container,
            Some(module.id),
            Some(submodule.id),
            "补过期用例",
        )
        .unwrap();
        insert_child(&conn, &in_submodule, "先写失败用例", Status::Todo);
        create_task(&conn, container, None, None, "整理文档").unwrap();

        create_task(&conn, Container::Todo, None, None, "待办里的").unwrap();
        create_task(&conn, Container::Project { id: kept }, None, None, "另一个项目的").unwrap();

        let plan = plan_delete_project(&conn, doomed).unwrap();
        assert_eq!(plan.task_count, 4, "含未分类任务与子任务");
        assert_eq!(list_tasks(&conn, container).unwrap().len(), 4, "plan 不写库");

        delete_project(&conn, doomed).unwrap();

        assert!(list_tasks(&conn, container).unwrap().is_empty());
        assert!(list_modules(&conn, doomed).unwrap().is_empty());
        assert_eq!(list_tasks(&conn, Container::Todo).unwrap().len(), 1);
        assert_eq!(
            list_tasks(&conn, Container::Project { id: kept })
                .unwrap()
                .len(),
            1
        );

        let projects = list_projects(&conn).unwrap();
        assert_eq!(
            projects.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["待办", "支付网关"]
        );
    }

    #[test]
    fn deleting_unknown_rows_errors() {
        let conn = conn();
        assert!(plan_delete_task(&conn, 999).is_err());
        assert!(delete_task(&conn, 999).is_err());
        assert!(plan_delete_node(&conn, NodeKind::Module, 999).is_err());
        assert!(plan_delete_node(&conn, NodeKind::Submodule, 999).is_err());
        assert!(delete_node(&conn, NodeKind::Module, 999).is_err());
        assert!(delete_node(&conn, NodeKind::Submodule, 999).is_err());
        assert!(plan_delete_project(&conn, 999).is_err());
        assert!(delete_project(&conn, 999).is_err());
    }

    // -----------------------------------------------------------------------
    // IPC 字段名契约（前端 api.ts 依赖这些名称，改了就两边一起改）
    // -----------------------------------------------------------------------

    #[test]
    fn task_wire_shape_is_camel_case() {
        let conn = conn();
        let task = seed_task(&conn);
        let json = serde_json::to_value(&task).unwrap();

        for key in [
            "projectId",
            "moduleId",
            "submoduleId",
            "parentTaskId",
            "sortOrder",
        ] {
            assert!(json.get(key).is_some(), "缺少 {key}");
        }
        assert_eq!(json["status"], "todo");
        assert_eq!(json["priority"], "mid");
    }

    #[test]
    fn status_plan_wire_shape_is_camel_case() {
        let json = serde_json::to_value(StatusPlan {
            affects: Vec::new(),
            needs_confirm: true,
            reason: "还有 1 个未完成子任务".to_string(),
        })
        .unwrap();

        assert_eq!(json["needsConfirm"], true);
        assert_eq!(json["reason"], "还有 1 个未完成子任务");
        assert!(json.get("needs_confirm").is_none(), "不能是 snake_case");
    }

    #[test]
    fn delete_plan_wire_shape_is_camel_case() {
        let json = serde_json::to_value(DeletePlan { task_count: 3 }).unwrap();
        assert_eq!(json["taskCount"], 3);
        assert!(json.get("task_count").is_none(), "不能是 snake_case");
    }

    #[test]
    fn node_kind_arg_deserializes_from_frontend() {
        assert_eq!(
            serde_json::from_str::<NodeKind>(r#""module""#).unwrap(),
            NodeKind::Module
        );
        assert_eq!(
            serde_json::from_str::<NodeKind>(r#""submodule""#).unwrap(),
            NodeKind::Submodule
        );
        assert!(serde_json::from_str::<NodeKind>(r#""project""#).is_err());
    }

    #[test]
    fn container_arg_deserializes_from_frontend() {
        let todo: Container = serde_json::from_str(r#"{"kind":"todo"}"#).unwrap();
        assert!(matches!(todo, Container::Todo));

        let project: Container = serde_json::from_str(r#"{"kind":"project","id":7}"#).unwrap();
        assert!(matches!(project, Container::Project { id: 7 }));
    }

    #[test]
    fn status_and_priority_args_deserialize() {
        assert_eq!(
            serde_json::from_str::<Status>(r#""doing""#).unwrap(),
            Status::Doing
        );
        assert_eq!(
            serde_json::from_str::<Priority>(r#""high""#).unwrap().as_str(),
            "high"
        );
        assert!(serde_json::from_str::<Status>(r#""blocked""#).is_err());
    }
}
