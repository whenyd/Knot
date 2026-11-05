//! 领域层：唯一的写入口，也是唯一允许表达"规则"的地方。
//!
//! 命令层只做参数解包 + 转发；SQL 不重复表达这里的规则（不用触发器），
//! 否则同一套规则会有两个实现，必然漂移。

use chrono::Local;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

/// 左栏最上方那个虚拟项目。它不是 `project` 表里的一行——
/// "我的待办"由 `task.project_id IS NULL` 表达，这里只是它的显示名。
pub const TODO_NAME: &str = "我的待办";

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

    /// 数字越大优先级越高。判断“子任务不能高过父任务”只比较这个。
    fn rank(self) -> u8 {
        match self {
            Priority::High => 3,
            Priority::Mid => 2,
            Priority::Low => 1,
        }
    }

    /// 严格高于 `self` 的档位，用于父任务降档时级联下调子任务。
    fn above(self) -> &'static [Priority] {
        match self {
            Priority::High => &[],
            Priority::Mid => &[Priority::High],
            Priority::Low => &[Priority::High, Priority::Mid],
        }
    }
}

/// 任务所在的容器：虚拟的「我的待办」，或某个真实项目。
///
/// 序列化成 `{ kind: "todo" }` / `{ kind: "project", id }`，前端原样回传。
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Container {
    Todo,
    Project { id: i64 },
}

impl Container {
    /// 对应的 `task.project_id`（NULL 即「我的待办」）。
    fn project_id(self) -> Option<i64> {
        match self {
            Container::Todo => None,
            Container::Project { id } => Some(id),
        }
    }
}

/// `move_task` 的目标位置：容器 + 两级分类（可空）。
///
/// 形状与 `create_task` 的归属参数一致，前端原样回传；合法性规则也共用
/// `require_task_target`，移动不会比新建多出一套判断。
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveTarget {
    pub container: Container,
    pub module_id: Option<i64>,
    pub submodule_id: Option<i64>,
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

/// 左栏的一项：项目，或虚拟的「我的待办」（`id` 为 `None`、`is_todo` 为 `true`）。
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
    /// 本地时间 `YYYY-MM-DDTHH:MM`；新建任务时写入，升级前的任务为 `None`。
    pub created_at: Option<String>,
    /// 本地时间 `YYYY-MM-DDTHH:MM`，`None` = 未记录；由状态变化自动维护。
    pub start_at: Option<String>,
    pub end_at: Option<String>,
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

/// `create_task` 的返回：任务本体 + 父任务被聚合自动改动时的轻提示。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskCreated {
    pub task: Task,
    /// 新建子任务把已完成的父任务推回进行中时，这里会有一条提示。
    pub notices: Vec<String>,
}

/// `delete_task` 的返回：删掉子任务后父任务被自动改动时的轻提示。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteOutcome {
    pub notices: Vec<String>,
}

/// 任务表的列顺序，读任务的地方只有这一份，避免 `row.get(i)` 的序号漂移。
const TASK_COLUMNS: &str = "id, project_id, module_id, submodule_id, parent_task_id, \
                            title, status, priority, note, sort_order, start_at, end_at, created_at";

// ---------------------------------------------------------------------------
// 读
// ---------------------------------------------------------------------------

/// 左栏全部条目：首项恒为「我的待办」，其余按 `sort_order`。
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

/// 项目的模块树（模块 + 其子模块）。"我的待办"没有模块，不调用这个命令。
pub fn list_modules(conn: &Connection, project_id: i64) -> Res<Vec<Module>> {
    let mut stmt = conn
        .prepare("SELECT id, name FROM module WHERE project_id = ?1 ORDER BY sort_order, id")
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
    let sql =
        format!("SELECT {TASK_COLUMNS} FROM task WHERE project_id IS ?1 ORDER BY sort_order, id");
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
    let sort_order = next_sort_order(conn, "module", "project_id = ?1", params![project_id])?;

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

/// 新建任务。`parent_task_id` 为 `Some` 时新建一层子任务。
///
/// 顶层归属规则：容器给项目，`module_id`/`submodule_id` 由调用方按"当前范围"给出，
/// 不给就是"默认"。优先级默认"中"，排序号取**同一组内**最大值 + 1（排在组内最后）。
///
/// 子任务的归属真相在父任务上：容器必须与父任务一致，`module_id`/`submodule_id`
/// 一律落 NULL（调用方传了也忽略）；排序号按父任务下的子任务单独排队。
/// 只允许一层——父任务自己已经是子任务时拒绝。优先级不能高过父任务：
/// 父任务为"低"时，新子任务也是"低"。
pub fn create_task(
    conn: &Connection,
    container: Container,
    module_id: Option<i64>,
    submodule_id: Option<i64>,
    parent_task_id: Option<i64>,
    title: &str,
) -> Res<TaskCreated> {
    let title = require_name(title, "任务文字")?;

    // 先把父任务校验完再开事务：校验失败时没有半截事务，读起来也更直接。
    let (parent_project, parent_priority) = match parent_task_id {
        None => (None, None),
        Some(parent_id) => {
            let parent = load_task(conn, parent_id)?;
            if parent.parent_task_id.is_some() {
                return Err("子任务不能再挂子任务".to_string());
            }
            if parent.project_id != container.project_id() {
                return Err("子任务必须与父任务在同一个容器".to_string());
            }
            (Some(parent.project_id), Some(parent.priority))
        }
    };

    let tx = conn.unchecked_transaction().map_err(db_err)?;

    let (project_id, module_id, submodule_id) = match parent_project {
        None => (
            require_task_target(&tx, container, module_id, submodule_id)?,
            module_id,
            submodule_id,
        ),
        // 子任务不写两级分类：归属由父任务表达，冗余一份只会漂移。
        Some(project_id) => (project_id, None, None),
    };

    let sort_order = match parent_task_id {
        None => next_sort_order(
            &tx,
            "task",
            "project_id IS ?1 AND module_id IS ?2 AND submodule_id IS ?3 \
             AND parent_task_id IS NULL",
            params![project_id, module_id, submodule_id],
        )?,
        Some(parent_id) => next_sort_order(&tx, "task", "parent_task_id = ?1", params![parent_id])?,
    };

    // 默认“中”，但父任务更低时跟着父任务，别让子任务一出生就违规。
    let priority = match parent_priority {
        Some(parent) if parent.rank() < Priority::Mid.rank() => parent,
        _ => Priority::Mid,
    };

    let created_at = now_local();
    tx.execute(
        "INSERT INTO task (project_id, module_id, submodule_id, parent_task_id, \
                           title, status, priority, sort_order, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            project_id,
            module_id,
            submodule_id,
            parent_task_id,
            title,
            // 默认值由领域层写出来，不靠 schema 的 DEFAULT：规则只有一处真相。
            Status::Todo.as_str(),
            priority.as_str(),
            sort_order,
            &created_at
        ],
    )
    .map_err(db_err)?;

    let task = Task {
        id: tx.last_insert_rowid(),
        project_id,
        module_id,
        submodule_id,
        parent_task_id,
        title: title.to_string(),
        status: Status::Todo,
        priority,
        note: String::new(),
        created_at: Some(created_at),
        start_at: None,
        end_at: None,
        sort_order,
    };

    // 新建子任务也是一个聚合事件：已完成的父任务要退回进行中。
    let mut notices = Vec::new();
    if let Some(parent_id) = parent_task_id
        && let Some(change) = aggregate_parent_status(&tx, parent_id)?
    {
        notices.push(change.notice);
    }

    tx.commit().map_err(db_err)?;

    Ok(TaskCreated { task, notices })
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
///
/// 子任务不能高过父任务：菜单不会给出非法选项，这里再兜底拒绝。
/// 父任务降档时，仍高于它的子任务在同一个事务里一并降档。
pub fn set_priority(conn: &Connection, id: i64, priority: Priority) -> Res<Task> {
    let mut task = load_task(conn, id)?;

    if let Some(parent_id) = task.parent_task_id {
        let parent = load_task(conn, parent_id)?;
        if priority.rank() > parent.priority.rank() {
            return Err("子任务优先级不能高于父任务".to_string());
        }
    }

    let tx = conn.unchecked_transaction().map_err(db_err)?;
    tx.execute(
        "UPDATE task SET priority = ?1 WHERE id = ?2",
        params![priority.as_str(), id],
    )
    .map_err(db_err)?;

    // 一次降档最多命中两个旧档位，逐条 UPDATE 比在 SQL 里重复一份档位顺序更好读。
    for old in priority.above() {
        tx.execute(
            "UPDATE task SET priority = ?1 WHERE parent_task_id = ?2 AND priority = ?3",
            params![priority.as_str(), id, old.as_str()],
        )
        .map_err(db_err)?;
    }
    tx.commit().map_err(db_err)?;

    task.priority = priority;
    Ok(task)
}

/// 改备注。**不 trim、不拒空**：备注允许任意文本（含换行与首尾空白），
/// 空串就是清空备注。表格里的单行摘要是展示层的事，落库保持原样。
pub fn set_note(conn: &Connection, id: i64, note: &str) -> Res<Task> {
    let mut task = load_task(conn, id)?;

    conn.execute("UPDATE task SET note = ?1 WHERE id = ?2", params![note, id])
        .map_err(db_err)?;

    task.note = note.to_string();
    Ok(task)
}

/// 把顶层任务移到另一个归类：容器、模块、子模块可以一起变。
///
/// 目标组的合法性完全交给 `require_task_target`（与新建同一套规则）；
/// 移动后排在目标组末尾。子任务不能单独移动——它的归属真相在父任务上，
/// 这里只把它的 `project_id` 跟着换成新容器。
pub fn move_task(conn: &Connection, id: i64, target: MoveTarget) -> Res<Task> {
    let mut task = load_task(conn, id)?;
    if task.parent_task_id.is_some() {
        return Err("子任务跟随父任务，不能单独移动".to_string());
    }

    let project_id = require_task_target(
        conn,
        target.container,
        target.module_id,
        target.submodule_id,
    )?;
    let tx = conn.unchecked_transaction().map_err(db_err)?;

    // 与 `create_task` 逐字相同的分组条件：移动后的排序号就是那个组的 MAX+1。
    let sort_order = next_sort_order(
        &tx,
        "task",
        "project_id IS ?1 AND module_id IS ?2 AND submodule_id IS ?3 \
         AND parent_task_id IS NULL",
        params![project_id, target.module_id, target.submodule_id],
    )?;

    tx.execute(
        "UPDATE task SET project_id = ?1, module_id = ?2, submodule_id = ?3, sort_order = ?4 \
          WHERE id = ?5",
        params![
            project_id,
            target.module_id,
            target.submodule_id,
            sort_order,
            id
        ],
    )
    .map_err(db_err)?;

    // 子任务跟随容器；两级分类保持 NULL，归属仍由父任务表达。
    tx.execute(
        "UPDATE task SET project_id = ?1 WHERE parent_task_id = ?2",
        params![project_id, id],
    )
    .map_err(db_err)?;

    tx.commit().map_err(db_err)?;

    task.project_id = project_id;
    task.module_id = target.module_id;
    task.submodule_id = target.submodule_id;
    task.sort_order = sort_order;
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
/// 如果目标本身是子任务，写完后触发父任务聚合（子全完成 → 父完成，
/// 否则父若是 done → 父退回 doing）。目标是父任务时（包括 cascade 写子任务）
/// 不向上聚合；手动改父任务永远不动子任务。
pub fn apply_set_status(
    conn: &Connection,
    id: i64,
    status: Status,
    cascade: bool,
) -> Res<StatusOutcome> {
    let mut task = load_task(conn, id)?;
    let parent_id = task.parent_task_id;
    let now = now_local();
    let tx = conn.unchecked_transaction().map_err(db_err)?;

    write_status(&tx, &mut task, status, &now)?;
    let mut changed = vec![task];
    let mut notices = Vec::new();

    if cascade && status == Status::Done {
        for mut child in unfinished_children(&tx, id)? {
            write_status(&tx, &mut child, Status::Done, &now)?;
            changed.push(child);
        }
    }

    // 只有目标本身是子任务才聚合；父任务的手动改动不产生提示。
    if let Some(parent_id) = parent_id
        && let Some(change) = aggregate_parent_status(&tx, parent_id)?
    {
        changed.push(change.task);
        notices.push(change.notice);
    }

    tx.commit().map_err(db_err)?;

    Ok(StatusOutcome { changed, notices })
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

/// 删除任务。删除与父任务聚合在同一个事务里：删掉最后一个子任务会让父任务
/// 按聚合规则变成已完成，并带回一条轻提示。删父任务只向下级联，不向上聚合。
pub fn delete_task(conn: &Connection, id: i64) -> Res<DeleteOutcome> {
    // 先记住归属：DELETE 之后父 id 就查不到了。
    let task = load_task(conn, id)?;
    let parent_id = task.parent_task_id;
    let tx = conn.unchecked_transaction().map_err(db_err)?;

    let removed = tx
        .execute("DELETE FROM task WHERE id = ?1", params![id])
        .map_err(db_err)?;
    if removed == 0 {
        return Err(format!("任务 {id} 不存在"));
    }

    let mut notices = Vec::new();
    if let Some(parent_id) = parent_id
        && let Some(change) = aggregate_parent_status(&tx, parent_id)?
    {
        notices.push(change.notice);
    }

    tx.commit().map_err(db_err)?;
    Ok(DeleteOutcome { notices })
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

/// 写状态，并按状态变化维护开始 / 结束时间。
///
/// 只有状态**真的改变**时才动时间，重复写同一状态不会覆盖已有时间：
/// - 进入 `doing`：开始时间为空则记当前时间；结束时间清空（继续做，尚未结束）
/// - 进入 `done`：记当前时间为结束时间
/// - 回到 `todo`：清空两项（这一轮作废，下次开始时重新记）
///
/// 聚合自动改动父任务时也走这里，所以自动完成 / 自动恢复同样会维护时间。
fn write_status(conn: &Connection, task: &mut Task, status: Status, now: &str) -> Res<()> {
    if status != task.status {
        match status {
            Status::Todo => {
                task.start_at = None;
                task.end_at = None;
            }
            Status::Doing => {
                if task.start_at.is_none() {
                    task.start_at = Some(now.to_string());
                }
                task.end_at = None;
            }
            Status::Done => {
                task.end_at = Some(now.to_string());
            }
        }
    }

    conn.execute(
        "UPDATE task SET status = ?1, start_at = ?2, end_at = ?3 WHERE id = ?4",
        params![status.as_str(), task.start_at, task.end_at, task.id],
    )
    .map_err(db_err)?;
    task.status = status;
    Ok(())
}

/// 状态变化时记录的本地时间，精度到分钟；格式与 `start_at` / `end_at` 一致。
fn now_local() -> String {
    Local::now().format("%Y-%m-%dT%H:%M").to_string()
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

/// 某个父任务下的全部子任务（一层，不递归）。
fn load_children(conn: &Connection, parent_id: i64) -> Res<Vec<Task>> {
    let sql = format!(
        "SELECT {TASK_COLUMNS} FROM task WHERE parent_task_id = ?1 ORDER BY sort_order, id"
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

/// 父任务被聚合自动改动时的结果：改完的任务 + 给用户看的一句话。
struct ParentChange {
    task: Task,
    notice: String,
}

/// 事件触发的父状态推导，只在父状态**真的被改写**时返回 `Some`。
///
/// 规则按字面实现：子任务全为 `done` → 父置 `done`；否则且父当前为 `done`
/// → 父置 `doing`；其余情况不动。零个子任务时 `all()` 为真，所以删除最后一个
/// 未完成子任务也会把父任务推到已完成——这是规格明确的推导结果，不是不变式。
fn aggregate_parent_status(conn: &Connection, parent_id: i64) -> Res<Option<ParentChange>> {
    let mut parent = load_task(conn, parent_id)?;
    let children = load_children(conn, parent_id)?;
    let all_done = children.iter().all(|child| child.status == Status::Done);

    let next = if all_done {
        if parent.status == Status::Done {
            return Ok(None);
        }
        Status::Done
    } else {
        if parent.status != Status::Done {
            return Ok(None);
        }
        Status::Doing
    };

    write_status(conn, &mut parent, next, &now_local())?;
    let notice = match next {
        Status::Done => format!("父任务「{}」已自动标记为已完成", parent.title),
        Status::Doing => format!("父任务「{}」已恢复为进行中", parent.title),
        Status::Todo => unreachable!("聚合只会写 done / doing"),
    };

    Ok(Some(ParentChange {
        task: parent,
        notice,
    }))
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
    start_at: Option<String>,
    end_at: Option<String>,
    created_at: Option<String>,
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
            created_at: self.created_at,
            start_at: self.start_at,
            end_at: self.end_at,
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
        start_at: row.get(10)?,
        end_at: row.get(11)?,
        created_at: row.get(12)?,
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

/// 任务归属的唯一校验：新建与移动都走这里。
///
/// 返回任务应落的 `project_id`（「我的待办」为 `None`）。规则：
/// 容器是项目时项目必须存在；带模块时模块必须属于该项目；
/// 「我的待办」不能带模块；带子模块时必须先有模块，且子模块属于该模块。
fn require_task_target(
    conn: &Connection,
    container: Container,
    module_id: Option<i64>,
    submodule_id: Option<i64>,
) -> Res<Option<i64>> {
    let project_id = container.project_id();
    match (project_id, module_id) {
        (Some(project_id), Some(module_id)) => {
            require_project(conn, project_id)?;
            require_module(conn, project_id, module_id)?;
        }
        // 「我的待办」没有模块，带模块归属是调用方的错
        (None, Some(_)) => return Err("「我的待办」里的任务不属于任何模块".to_string()),
        (Some(project_id), None) => require_project(conn, project_id)?,
        (None, None) => {}
    }
    if let Some(submodule_id) = submodule_id {
        let module_id = module_id.ok_or("子模块必须附属于一个模块")?;
        require_submodule(conn, module_id, submodule_id)?;
    }
    Ok(project_id)
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
    let sql = format!("SELECT COALESCE(MAX(sort_order), -1) + 1 FROM {table} WHERE {where_clause}");
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
            None,
            "处理并发刷新",
        )
        .unwrap()
        .task;

        assert_eq!(task.project_id, Some(project_id));
        assert_eq!(task.module_id, None, "新任务落在默认分组");
        assert_eq!(task.submodule_id, None);
        assert_eq!(task.priority.as_str(), "mid", "默认优先级为中");
        assert_eq!(task.status.as_str(), "todo");
        assert_eq!(task.sort_order, 0);

        let tasks = list_tasks(&conn, Container::Project { id: project_id }).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "处理并发刷新");
    }

    /// 「我的待办」不是一行项目：它由 `project_id IS NULL` 表达，且不混进项目列表。
    #[test]
    fn todo_tasks_stay_out_of_projects() {
        let conn = conn();
        let project = create_project(&conn, "支付网关").unwrap();
        create_task(
            &conn,
            Container::Todo,
            None,
            None,
            None,
            "看一下 CI 偶发失败",
        )
        .unwrap();

        let projects = list_projects(&conn).unwrap();
        assert!(projects[0].is_todo, "首项恒为「我的待办」");
        assert_eq!(projects[0].id, None);
        assert_eq!(projects[0].task_count, 1);

        let real = projects.iter().find(|p| p.id == project.id).unwrap();
        assert_eq!(real.task_count, 0, "我的待办里的任务不算进项目");

        let tasks = list_tasks(&conn, Container::Todo).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].project_id, None);
    }

    /// 组内排序号递增：新任务排在组内最后。
    #[test]
    fn sort_order_appends_within_group() {
        let conn = conn();
        let project = create_project(&conn, "文档站").unwrap();
        let container = Container::Project {
            id: project.id.unwrap(),
        };

        assert_eq!(
            create_task(&conn, container, None, None, None, "a")
                .unwrap()
                .task
                .sort_order,
            0
        );
        assert_eq!(
            create_task(&conn, container, None, None, None, "b")
                .unwrap()
                .task
                .sort_order,
            1
        );
        // 不同容器各排各的
        assert_eq!(
            create_task(&conn, Container::Todo, None, None, None, "c")
                .unwrap()
                .task
                .sort_order,
            0
        );
    }

    /// 空白文字在前端会被拦，后端也必须自己拦——不能只信任前端。
    #[test]
    fn rejects_blank_names() {
        let conn = conn();
        assert!(create_project(&conn, "   ").is_err());
        assert!(create_task(&conn, Container::Todo, None, None, None, "\t\n").is_err());
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
        assert!(create_task(&conn, Container::Project { id: 999 }, None, None, None, "x").is_err());
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
            None,
            "登录限流",
        )
        .unwrap()
        .task;

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
            None,
            "补过期用例",
        )
        .unwrap()
        .task;

        assert_eq!(task.module_id, Some(module.id));
        assert_eq!(task.submodule_id, Some(submodule));
    }

    /// 排序号按**同一组**算：模块里的任务与默认的任务各排各的。
    #[test]
    fn sort_order_is_per_group() {
        let conn = conn();
        let project = create_project(&conn, "todolist-tr").unwrap();
        let project_id = project.id.unwrap();
        let container = Container::Project { id: project_id };
        let module = create_module(&conn, project_id, "认证").unwrap();

        assert_eq!(
            create_task(&conn, container, None, None, None, "默认的")
                .unwrap()
                .task
                .sort_order,
            0
        );
        assert_eq!(
            create_task(&conn, container, Some(module.id), None, None, "模块里的")
                .unwrap()
                .task
                .sort_order,
            0,
            "另一个组从头数起"
        );
        assert_eq!(
            create_task(
                &conn,
                container,
                Some(module.id),
                None,
                None,
                "模块里的第二条"
            )
            .unwrap()
            .task
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

        assert!(
            create_task(
                &conn,
                Container::Project { id: b },
                Some(module.id),
                None,
                None,
                "x"
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_submodule_from_other_module() {
        let conn = conn();
        let project_id = create_project(&conn, "todolist-tr").unwrap().id.unwrap();
        let auth = create_module(&conn, project_id, "认证").unwrap();
        let billing = create_module(&conn, project_id, "计费").unwrap();
        let submodule = seed_submodule(&conn, auth.id, "token 刷新");

        assert!(
            create_task(
                &conn,
                Container::Project { id: project_id },
                Some(billing.id),
                Some(submodule),
                None,
                "x"
            )
            .is_err()
        );
    }

    /// 「我的待办」没有模块，带模块归属是调用方的错，不是默默把它当默认分组。
    #[test]
    fn rejects_module_on_todo_container() {
        let conn = conn();
        let project_id = create_project(&conn, "todolist-tr").unwrap().id.unwrap();
        let module = create_module(&conn, project_id, "认证").unwrap();

        assert!(create_task(&conn, Container::Todo, Some(module.id), None, None, "x").is_err());
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
            auth_now
                .submodules
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
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
        assert_eq!(
            load_submodule(&conn, submodule.id).unwrap().name,
            "token 刷新"
        );
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
            None,
            "登录限流",
        )
        .unwrap()
        .task;

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

        assert!(
            create_task(
                &conn,
                Container::Project { id: project_id },
                None,
                Some(submodule),
                None,
                "x"
            )
            .is_err()
        );
    }

    // -----------------------------------------------------------------------
    // 可编辑三件套
    // -----------------------------------------------------------------------

    /// 直接往库里塞一条子任务：测试需要任意初始状态时用，正常路径走 `create_child`。
    fn insert_child(conn: &Connection, parent: &Task, title: &str, status: Status) -> i64 {
        conn.execute(
            "INSERT INTO task (project_id, parent_task_id, title, status, priority) \
             VALUES (?1, ?2, ?3, ?4, 'mid')",
            params![parent.project_id, parent.id, title, status.as_str()],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn container_of(task: &Task) -> Container {
        match task.project_id {
            None => Container::Todo,
            Some(id) => Container::Project { id },
        }
    }

    /// 走正式命令建一层子任务（父任务默认 `todo`）。
    fn create_child(conn: &Connection, parent: &Task, title: &str) -> TaskCreated {
        create_task(
            conn,
            container_of(parent),
            None,
            None,
            Some(parent.id),
            title,
        )
        .unwrap()
    }

    fn seed_task(conn: &Connection) -> Task {
        let container = Container::Todo;
        create_task(conn, container, None, None, None, "实现 token 轮换")
            .unwrap()
            .task
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

    #[test]
    fn subtask_priority_cannot_exceed_parent() {
        let conn = conn();
        let parent = seed_task(&conn); // 默认 mid
        let child = create_child(&conn, &parent, "子任务").task; // 默认 mid

        let error = set_priority(&conn, child.id, Priority::High).unwrap_err();
        assert!(error.contains("不能高于父任务"), "{error}");
        assert_eq!(load_task(&conn, child.id).unwrap().priority.as_str(), "mid");

        // 同级与降档都允许。
        set_priority(&conn, child.id, Priority::Mid).unwrap();
        set_priority(&conn, child.id, Priority::Low).unwrap();
        assert_eq!(load_task(&conn, child.id).unwrap().priority.as_str(), "low");
    }

    #[test]
    fn new_subtask_is_capped_by_low_parent() {
        let conn = conn();
        let parent = seed_task(&conn);

        set_priority(&conn, parent.id, Priority::Low).unwrap();
        let child = create_child(&conn, &parent, "低父的子任务").task;
        assert_eq!(
            child.priority.as_str(),
            "low",
            "父任务为低时不能再生出中/高子任务"
        );

        set_priority(&conn, parent.id, Priority::High).unwrap();
        let another = create_child(&conn, &parent, "高父的子任务").task;
        assert_eq!(another.priority.as_str(), "mid", "默认档位仍是中");
    }

    #[test]
    fn lowering_parent_priority_cascades_to_higher_children() {
        let conn = conn();
        let parent = seed_task(&conn);
        set_priority(&conn, parent.id, Priority::High).unwrap();

        let high = create_child(&conn, &parent, "高子").task;
        set_priority(&conn, high.id, Priority::High).unwrap();
        let mid = create_child(&conn, &parent, "中子").task;
        let low = create_child(&conn, &parent, "低子").task;
        set_priority(&conn, low.id, Priority::Low).unwrap();

        set_priority(&conn, parent.id, Priority::Mid).unwrap();
        assert_eq!(load_task(&conn, high.id).unwrap().priority.as_str(), "mid");
        assert_eq!(load_task(&conn, mid.id).unwrap().priority.as_str(), "mid");
        assert_eq!(load_task(&conn, low.id).unwrap().priority.as_str(), "low");

        set_priority(&conn, parent.id, Priority::Low).unwrap();
        assert_eq!(load_task(&conn, high.id).unwrap().priority.as_str(), "low");
        assert_eq!(load_task(&conn, mid.id).unwrap().priority.as_str(), "low");
        assert_eq!(load_task(&conn, low.id).unwrap().priority.as_str(), "low");
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
    // 子任务（一层）与父状态聚合
    // -----------------------------------------------------------------------

    /// 子任务继承父任务的项目，两级分类恒为 NULL（传了也忽略）。
    #[test]
    fn subtask_inherits_project_and_nulls_classification() {
        let conn = conn();
        let project_id = create_project(&conn, "todolist-tr").unwrap().id.unwrap();
        let container = Container::Project { id: project_id };
        let module = create_module(&conn, project_id, "认证").unwrap();
        let parent = create_task(&conn, container, Some(module.id), None, None, "父任务")
            .unwrap()
            .task;

        let created = create_task(
            &conn,
            container,
            Some(module.id),
            None,
            Some(parent.id),
            "子任务",
        )
        .unwrap();
        let child = created.task;

        assert_eq!(child.parent_task_id, Some(parent.id));
        assert_eq!(child.project_id, Some(project_id), "跟随父任务的项目");
        assert_eq!(child.module_id, None, "归属真相在父任务，不冗余分类");
        assert_eq!(child.submodule_id, None);
        assert_eq!(child.status.as_str(), "todo");
        assert_eq!(child.priority.as_str(), "mid");
        assert!(created.notices.is_empty(), "父任务本来就是未完成");
    }

    #[test]
    fn subtask_in_todo_inherits_null_project() {
        let conn = conn();
        let parent = seed_task(&conn);
        let child = create_child(&conn, &parent, "子任务").task;

        assert_eq!(child.project_id, None);
        assert_eq!(child.module_id, None);
        assert_eq!(child.submodule_id, None);
        assert_eq!(child.parent_task_id, Some(parent.id));
    }

    /// 只允许一层：子任务不能再挂子任务。
    #[test]
    fn subtask_rejects_grandchild() {
        let conn = conn();
        let parent = seed_task(&conn);
        let child = create_child(&conn, &parent, "子任务").task;

        let error =
            create_task(&conn, Container::Todo, None, None, Some(child.id), "孙任务").unwrap_err();
        assert!(error.contains("子任务不能再挂子任务"), "{error}");
        assert_eq!(list_tasks(&conn, Container::Todo).unwrap().len(), 2);
    }

    #[test]
    fn subtask_rejects_unknown_parent() {
        let conn = conn();
        let error =
            create_task(&conn, Container::Todo, None, None, Some(999), "子任务").unwrap_err();
        assert!(error.contains("不存在"), "{error}");
        assert!(list_tasks(&conn, Container::Todo).unwrap().is_empty());
    }

    #[test]
    fn subtask_rejects_parent_in_another_container() {
        let conn = conn();
        let a = create_project(&conn, "A").unwrap().id.unwrap();
        let b = create_project(&conn, "B").unwrap().id.unwrap();
        let parent = create_task(
            &conn,
            Container::Project { id: a },
            None,
            None,
            None,
            "父任务",
        )
        .unwrap()
        .task;

        let error = create_task(
            &conn,
            Container::Project { id: b },
            None,
            None,
            Some(parent.id),
            "子任务",
        )
        .unwrap_err();
        assert!(error.contains("同一个容器"), "{error}");
        assert_eq!(
            list_tasks(&conn, Container::Project { id: b })
                .unwrap()
                .len(),
            0
        );
    }

    /// 排序号按父任务各自排队，不会和顶层任务或其他父任务的子任务混在一起。
    #[test]
    fn subtask_sort_order_is_per_parent() {
        let conn = conn();
        let first = create_task(&conn, Container::Todo, None, None, None, "父一")
            .unwrap()
            .task;
        let second = create_task(&conn, Container::Todo, None, None, None, "父二")
            .unwrap()
            .task;

        assert_eq!(create_child(&conn, &first, "a").task.sort_order, 0);
        assert_eq!(create_child(&conn, &first, "b").task.sort_order, 1);
        assert_eq!(create_child(&conn, &second, "c").task.sort_order, 0);
    }

    /// 事件触发 1：在已完成的父任务下新增未完成子任务 → 父退回进行中。
    #[test]
    fn creating_subtask_on_done_parent_reopens_it_with_notice() {
        let conn = conn();
        let parent = seed_task(&conn);
        apply_set_status(&conn, parent.id, Status::Done, false).unwrap();

        let created = create_child(&conn, &parent, "新子任务");

        assert_eq!(created.notices.len(), 1);
        assert!(
            created.notices[0].contains("恢复为进行中"),
            "{:?}",
            created.notices
        );
        assert_eq!(created.task.status.as_str(), "todo");
        assert_eq!(
            load_task(&conn, parent.id).unwrap().status.as_str(),
            "doing"
        );
    }

    #[test]
    fn creating_subtask_on_open_parent_changes_nothing() {
        let conn = conn();
        let parent = seed_task(&conn);

        let created = create_child(&conn, &parent, "子任务");

        assert!(created.notices.is_empty());
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "todo");
    }

    /// 事件触发 2：最后一个未完成子任务完成 → 父完成 + 轻提示。
    #[test]
    fn last_child_done_completes_parent_with_notice() {
        let conn = conn();
        let parent = seed_task(&conn);
        let first = create_child(&conn, &parent, "子任务一").task;
        let second = create_child(&conn, &parent, "子任务二").task;

        let outcome = apply_set_status(&conn, first.id, Status::Done, false).unwrap();
        assert!(outcome.notices.is_empty(), "还有未完成子任务，父不动");
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "todo");

        let outcome = apply_set_status(&conn, second.id, Status::Done, false).unwrap();
        assert_eq!(outcome.notices.len(), 1);
        assert!(
            outcome.notices[0].contains("已完成"),
            "{:?}",
            outcome.notices
        );
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "done");
        let aggregated = outcome.changed.last().expect("自动改动的父任务也要带回");
        assert_eq!(aggregated.id, parent.id);
        assert_eq!(aggregated.status.as_str(), "done");
    }

    #[test]
    fn non_last_child_done_leaves_parent_alone() {
        let conn = conn();
        let parent = seed_task(&conn);
        let first = create_child(&conn, &parent, "子任务一").task;
        create_child(&conn, &parent, "子任务二");

        let outcome = apply_set_status(&conn, first.id, Status::Done, false).unwrap();

        assert!(outcome.notices.is_empty());
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "todo");
    }

    /// 子任务退回：父若已完成 → 退回进行中；其他子任务不动。
    #[test]
    fn child_revert_reopens_done_parent_with_notice() {
        let conn = conn();
        let parent = seed_task(&conn);
        let first = create_child(&conn, &parent, "子任务一").task;
        let second = create_child(&conn, &parent, "子任务二").task;
        apply_set_status(&conn, first.id, Status::Done, false).unwrap();
        apply_set_status(&conn, second.id, Status::Done, false).unwrap();
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "done");

        let outcome = apply_set_status(&conn, first.id, Status::Todo, false).unwrap();

        assert_eq!(outcome.notices.len(), 1);
        assert!(
            outcome.notices[0].contains("恢复为进行中"),
            "{:?}",
            outcome.notices
        );
        assert_eq!(
            load_task(&conn, parent.id).unwrap().status.as_str(),
            "doing"
        );
        assert_eq!(load_task(&conn, second.id).unwrap().status.as_str(), "done");
    }

    #[test]
    fn child_revert_with_unfinished_sibling_does_not_touch_parent() {
        let conn = conn();
        let parent = seed_task(&conn);
        let first = create_child(&conn, &parent, "子任务一").task;
        create_child(&conn, &parent, "子任务二");
        apply_set_status(&conn, first.id, Status::Done, false).unwrap();

        let outcome = apply_set_status(&conn, first.id, Status::Doing, false).unwrap();

        assert!(outcome.notices.is_empty());
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "todo");
    }

    /// 手动退回父任务：子任务不动，也不产生提示。
    #[test]
    fn manual_parent_revert_leaves_children_and_has_no_notice() {
        let conn = conn();
        let parent = seed_task(&conn);
        let first = create_child(&conn, &parent, "子任务一").task;
        let second = create_child(&conn, &parent, "子任务二").task;
        apply_set_status(&conn, first.id, Status::Done, false).unwrap();
        apply_set_status(&conn, second.id, Status::Done, false).unwrap();
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "done");

        let outcome = apply_set_status(&conn, parent.id, Status::Todo, false).unwrap();

        assert!(outcome.notices.is_empty(), "用户自己点的这一下不产生提示");
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "todo");
        assert_eq!(load_task(&conn, first.id).unwrap().status.as_str(), "done");
        assert_eq!(load_task(&conn, second.id).unwrap().status.as_str(), "done");
    }

    /// 聚合是字面规则：没有子任务时 `all()` 为真，父任务会被推到已完成。
    #[test]
    fn aggregating_empty_child_set_marks_parent_done() {
        let conn = conn();
        let parent = seed_task(&conn);

        let change = aggregate_parent_status(&conn, parent.id)
            .unwrap()
            .expect("空子任务集合应触发聚合");

        assert_eq!(change.task.status.as_str(), "done");
        assert!(change.notice.contains("已完成"), "{}", change.notice);
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "done");
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

        let outcome = delete_task(&conn, task.id).unwrap();
        assert!(outcome.notices.is_empty(), "删父任务只向下级联，不向上聚合");
        assert!(load_task(&conn, task.id).is_err());
        assert!(load_task(&conn, open).is_err());
        assert!(load_task(&conn, done).is_err());
    }

    /// 事件触发 3：删掉最后一个未完成子任务 → 父任务完成 + 轻提示。
    #[test]
    fn deleting_last_unfinished_child_completes_parent_with_notice() {
        let conn = conn();
        let parent = seed_task(&conn);
        let child = create_child(&conn, &parent, "子任务").task;

        let outcome = delete_task(&conn, child.id).unwrap();

        assert_eq!(outcome.notices.len(), 1);
        assert!(
            outcome.notices[0].contains("已完成"),
            "{:?}",
            outcome.notices
        );
        assert!(load_task(&conn, child.id).is_err());
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "done");
    }

    #[test]
    fn deleting_child_with_unfinished_sibling_does_not_touch_parent() {
        let conn = conn();
        let parent = seed_task(&conn);
        let first = create_child(&conn, &parent, "子任务一").task;
        let second = create_child(&conn, &parent, "子任务二").task;

        let outcome = delete_task(&conn, first.id).unwrap();

        assert!(outcome.notices.is_empty());
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "todo");
        assert_eq!(load_task(&conn, second.id).unwrap().status.as_str(), "todo");
    }

    /// 父任务已经完成时删掉已完成的子任务：状态不变，不重复提示。
    #[test]
    fn deleting_done_child_from_done_parent_emits_no_notice() {
        let conn = conn();
        let parent = seed_task(&conn);
        let child = create_child(&conn, &parent, "子任务").task;
        apply_set_status(&conn, child.id, Status::Done, false).unwrap();
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "done");

        let outcome = delete_task(&conn, child.id).unwrap();

        assert!(outcome.notices.is_empty());
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "done");
    }

    /// 删模块：子模块、模块里的任务、子模块里的任务及其子任务一起消失，
    /// 默认任务与别的项目不受影响。
    #[test]
    fn delete_module_counts_submodule_tasks_and_cascades() {
        let conn = conn();
        let project_id = create_project(&conn, "todolist-tr").unwrap().id.unwrap();
        let container = Container::Project { id: project_id };
        let module = create_module(&conn, project_id, "认证").unwrap();
        let submodule = create_submodule(&conn, module.id, "token 刷新").unwrap();

        let in_module = create_task(&conn, container, Some(module.id), None, None, "登录限流")
            .unwrap()
            .task;
        let in_submodule = create_task(
            &conn,
            container,
            Some(module.id),
            Some(submodule.id),
            None,
            "补过期用例",
        )
        .unwrap()
        .task;
        let child = insert_child(&conn, &in_submodule, "先写失败用例", Status::Todo);
        let outside = create_task(&conn, container, None, None, None, "整理文档")
            .unwrap()
            .task;

        let plan = plan_delete_node(&conn, NodeKind::Module, module.id).unwrap();
        assert_eq!(plan.task_count, 3, "模块任务 + 子模块任务 + 子任务");
        assert_eq!(
            list_tasks(&conn, container).unwrap().len(),
            4,
            "plan 不写库"
        );

        delete_node(&conn, NodeKind::Module, module.id).unwrap();

        let remaining = list_tasks(&conn, container).unwrap();
        assert_eq!(
            remaining
                .iter()
                .map(|t| t.title.as_str())
                .collect::<Vec<_>>(),
            vec!["整理文档"]
        );
        assert!(
            list_modules(&conn, project_id).unwrap().is_empty(),
            "子模块随模块删除"
        );
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

        let in_module = create_task(&conn, container, Some(module.id), None, None, "登录限流")
            .unwrap()
            .task;
        let in_submodule = create_task(
            &conn,
            container,
            Some(module.id),
            Some(submodule.id),
            None,
            "补过期用例",
        )
        .unwrap()
        .task;
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

    /// 删项目：项目里的所有任务（含默认、子任务）与模块树全部消失，
    /// 我的待办和别的项目一条都不动。
    #[test]
    fn delete_project_counts_all_tasks_and_cascades() {
        let conn = conn();
        let doomed = create_project(&conn, "todolist-tr").unwrap().id.unwrap();
        let kept = create_project(&conn, "支付网关").unwrap().id.unwrap();
        let container = Container::Project { id: doomed };

        let module = create_module(&conn, doomed, "认证").unwrap();
        let submodule = create_submodule(&conn, module.id, "token 刷新").unwrap();
        create_task(&conn, container, Some(module.id), None, None, "登录限流").unwrap();
        let in_submodule = create_task(
            &conn,
            container,
            Some(module.id),
            Some(submodule.id),
            None,
            "补过期用例",
        )
        .unwrap()
        .task;
        insert_child(&conn, &in_submodule, "先写失败用例", Status::Todo);
        create_task(&conn, container, None, None, None, "整理文档").unwrap();

        create_task(&conn, Container::Todo, None, None, None, "我的待办里的").unwrap();
        create_task(
            &conn,
            Container::Project { id: kept },
            None,
            None,
            None,
            "另一个项目的",
        )
        .unwrap();

        let plan = plan_delete_project(&conn, doomed).unwrap();
        assert_eq!(plan.task_count, 4, "含默认任务与子任务");
        assert_eq!(
            list_tasks(&conn, container).unwrap().len(),
            4,
            "plan 不写库"
        );

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
            vec!["我的待办", "支付网关"]
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
    // 备注
    // -----------------------------------------------------------------------

    /// 备注允许换行与连续空白，落库保持原样——摘要的单行化是展示层的事。
    #[test]
    fn set_note_keeps_multiline_text() {
        let conn = conn();
        let task = seed_task(&conn);
        let note = "第一行\n第二行\n\n缩进  空格";

        let updated = set_note(&conn, task.id, note).unwrap();
        assert_eq!(updated.note, note);
        assert_eq!(load_task(&conn, task.id).unwrap().note, note);
    }

    /// 空串是"清空"，不是"拒绝"。
    #[test]
    fn set_note_can_clear() {
        let conn = conn();
        let task = seed_task(&conn);
        set_note(&conn, task.id, "先写点东西").unwrap();

        let cleared = set_note(&conn, task.id, "").unwrap();
        assert_eq!(cleared.note, "");
        assert_eq!(load_task(&conn, task.id).unwrap().note, "");
    }

    #[test]
    fn set_note_unknown_task_errors() {
        let conn = conn();
        assert!(set_note(&conn, 999, "x").is_err());
    }

    // -----------------------------------------------------------------------
    // 开始 / 结束时间：由状态变化自动维护
    // -----------------------------------------------------------------------

    /// 时间格式固定为 16 位本地时间，返回引用方便继续比较先后。
    fn assert_local_time(value: &Option<String>) -> &str {
        let raw = value.as_deref().expect("状态变化后应自动记录时间");
        let bytes = raw.as_bytes();
        assert_eq!(bytes.len(), 16, "{raw}");
        assert!(
            (bytes[4] == b'-' && bytes[7] == b'-' && bytes[10] == b'T' && bytes[13] == b':'),
            "{raw}"
        );
        assert!(
            bytes
                .iter()
                .enumerate()
                .all(|(i, byte)| matches!(i, 4 | 7 | 10 | 13) || byte.is_ascii_digit()),
            "{raw}"
        );
        raw
    }

    #[test]
    fn new_task_records_created_at_only() {
        let conn = conn();
        let task = seed_task(&conn);
        assert_local_time(&task.created_at);
        assert_eq!(task.start_at, None);
        assert_eq!(task.end_at, None);
    }

    /// 进入进行中：记开始时间，结束时间保持为空。
    #[test]
    fn moving_to_doing_records_start_only() {
        let conn = conn();
        let task = seed_task(&conn);

        let outcome = apply_set_status(&conn, task.id, Status::Doing, false).unwrap();
        let changed = &outcome.changed[0];
        assert_local_time(&changed.start_at);
        assert_eq!(changed.end_at, None);

        let loaded = load_task(&conn, task.id).unwrap();
        assert_local_time(&loaded.start_at);
        assert_eq!(loaded.end_at, None);
    }

    /// 进行中 → 已完成：保留开始时间，记结束时间，且结束不早于开始。
    #[test]
    fn completing_records_end_and_keeps_start() {
        let conn = conn();
        let task = seed_task(&conn);

        let doing = apply_set_status(&conn, task.id, Status::Doing, false)
            .unwrap()
            .changed[0]
            .clone();
        let start = doing.start_at.clone();

        let outcome = apply_set_status(&conn, task.id, Status::Done, false).unwrap();
        let changed = &outcome.changed[0];
        assert_eq!(changed.start_at, start);
        let end = assert_local_time(&changed.end_at);
        assert!(
            end >= start.as_deref().unwrap(),
            "结束 {end} 不应早于开始 {:?}",
            start
        );
    }

    /// 退回未完成：两项一起清空，下一轮重新开始。
    #[test]
    fn returning_to_todo_clears_both_times() {
        let conn = conn();
        let task = seed_task(&conn);

        apply_set_status(&conn, task.id, Status::Doing, false).unwrap();
        apply_set_status(&conn, task.id, Status::Done, false).unwrap();
        let cleared = apply_set_status(&conn, task.id, Status::Todo, false)
            .unwrap()
            .changed[0]
            .clone();
        assert_eq!(cleared.start_at, None);
        assert_eq!(cleared.end_at, None);

        // 再进入进行中，是全新的一轮。
        let again = apply_set_status(&conn, task.id, Status::Doing, false)
            .unwrap()
            .changed[0]
            .clone();
        assert_local_time(&again.start_at);
        assert_eq!(again.end_at, None);
    }

    /// 重复写入同一个状态不改写已有时间。
    #[test]
    fn rewriting_same_status_keeps_times() {
        let conn = conn();
        let task = seed_task(&conn);

        apply_set_status(&conn, task.id, Status::Doing, false).unwrap();
        let done = apply_set_status(&conn, task.id, Status::Done, false)
            .unwrap()
            .changed[0]
            .clone();
        let end = done.end_at.clone();

        let again = apply_set_status(&conn, task.id, Status::Done, false)
            .unwrap()
            .changed[0]
            .clone();
        assert_eq!(again.start_at, done.start_at);
        assert_eq!(again.end_at, end);
    }

    /// 级联完成：未开始的子任务只记结束，进行中的子任务保有自己的开始时间。
    #[test]
    fn cascade_done_stamps_children() {
        let conn = conn();
        let parent = seed_task(&conn);
        let started = create_child(&conn, &parent, "已开始").task;
        let untouched = create_child(&conn, &parent, "未开始").task;
        apply_set_status(&conn, started.id, Status::Doing, false).unwrap();
        let started_at = load_task(&conn, started.id).unwrap().start_at;

        apply_set_status(&conn, parent.id, Status::Done, true).unwrap();

        let started_after = load_task(&conn, started.id).unwrap();
        assert_eq!(started_after.start_at, started_at);
        assert_local_time(&started_after.end_at);

        let untouched_after = load_task(&conn, untouched.id).unwrap();
        assert_eq!(untouched_after.start_at, None);
        assert_local_time(&untouched_after.end_at);
    }

    /// 子任务全部完成触发父任务自动完成，父任务也要落下结束时间。
    #[test]
    fn aggregate_done_stamps_parent_end() {
        let conn = conn();
        let parent = seed_task(&conn);
        let child = create_child(&conn, &parent, "子任务").task;

        apply_set_status(&conn, child.id, Status::Done, false).unwrap();

        let parent_after = load_task(&conn, parent.id).unwrap();
        assert_eq!(parent_after.status.as_str(), "done");
        assert_local_time(&parent_after.end_at);
    }

    /// 子任务被退回导致父任务自动恢复进行中：清结束时间，保留开始时间。
    #[test]
    fn aggregate_reopen_clears_parent_end_and_keeps_start() {
        let conn = conn();
        let parent = seed_task(&conn);
        let child = create_child(&conn, &parent, "子任务").task;

        apply_set_status(&conn, parent.id, Status::Doing, false).unwrap();
        let start = load_task(&conn, parent.id).unwrap().start_at;
        apply_set_status(&conn, child.id, Status::Done, false).unwrap();
        assert_local_time(&load_task(&conn, parent.id).unwrap().end_at);

        apply_set_status(&conn, child.id, Status::Todo, false).unwrap();

        let reopened = load_task(&conn, parent.id).unwrap();
        assert_eq!(reopened.status.as_str(), "doing");
        assert_eq!(reopened.start_at, start);
        assert_eq!(reopened.end_at, None);
    }

    // -----------------------------------------------------------------------
    // 归类移动
    // -----------------------------------------------------------------------

    /// 移动后落在目标组末尾：排序号是目标组已有的 MAX+1，不是原值。
    #[test]
    fn move_task_to_module_appends_in_target_group() {
        let conn = conn();
        let project_id = create_project(&conn, "todolist-tr").unwrap().id.unwrap();
        let container = Container::Project { id: project_id };
        let module = create_module(&conn, project_id, "认证").unwrap();
        let existing = create_task(
            &conn,
            container,
            Some(module.id),
            None,
            None,
            "模块里的第一条",
        )
        .unwrap()
        .task;
        let task = create_task(&conn, container, None, None, None, "默认的")
            .unwrap()
            .task;
        assert_eq!(task.sort_order, 0, "默认组从头数起");
        assert_eq!(existing.sort_order, 0);

        let moved = move_task(
            &conn,
            task.id,
            MoveTarget {
                container,
                module_id: Some(module.id),
                submodule_id: None,
            },
        )
        .unwrap();

        assert_eq!(moved.project_id, Some(project_id));
        assert_eq!(moved.module_id, Some(module.id));
        assert_eq!(moved.submodule_id, None);
        assert_eq!(moved.sort_order, existing.sort_order + 1);

        let after = load_task(&conn, task.id).unwrap();
        assert_eq!(after.module_id, Some(module.id));
        assert_eq!(after.sort_order, 1);
    }

    /// 跨项目移动：父任务换到 B 的模块，子任务只跟容器（分类仍由父任务表达）。
    #[test]
    fn move_task_across_projects_moves_children_project() {
        let conn = conn();
        let a = create_project(&conn, "A").unwrap().id.unwrap();
        let b = create_project(&conn, "B").unwrap().id.unwrap();
        let module_b = create_module(&conn, b, "认证").unwrap();

        let parent = create_task(
            &conn,
            Container::Project { id: a },
            None,
            None,
            None,
            "父任务",
        )
        .unwrap()
        .task;
        let child = insert_child(&conn, &parent, "子任务", Status::Todo);

        move_task(
            &conn,
            parent.id,
            MoveTarget {
                container: Container::Project { id: b },
                module_id: Some(module_b.id),
                submodule_id: None,
            },
        )
        .unwrap();

        let parent_after = load_task(&conn, parent.id).unwrap();
        assert_eq!(parent_after.project_id, Some(b));
        assert_eq!(parent_after.module_id, Some(module_b.id));

        let child_after = load_task(&conn, child).unwrap();
        assert_eq!(child_after.project_id, Some(b), "子任务跟着父任务换容器");
        assert_eq!(child_after.module_id, None);
        assert_eq!(child_after.submodule_id, None);
    }

    /// 移回「我的待办」等于清空三级分类；子任务的容器也一起清空。
    #[test]
    fn move_task_to_todo_clears_classification() {
        let conn = conn();
        let project_id = create_project(&conn, "todolist-tr").unwrap().id.unwrap();
        let module = create_module(&conn, project_id, "认证").unwrap();
        let task = create_task(
            &conn,
            Container::Project { id: project_id },
            Some(module.id),
            None,
            None,
            "登录限流",
        )
        .unwrap()
        .task;
        let child = insert_child(&conn, &task, "子任务", Status::Todo);

        let moved = move_task(
            &conn,
            task.id,
            MoveTarget {
                container: Container::Todo,
                module_id: None,
                submodule_id: None,
            },
        )
        .unwrap();

        assert_eq!(moved.project_id, None);
        assert_eq!(moved.module_id, None);
        assert_eq!(moved.submodule_id, None);
        assert_eq!(load_task(&conn, child).unwrap().project_id, None);
    }

    /// 非法目标全部拒绝，且被移动的任务一个字段都不变。
    #[test]
    fn move_task_rejects_invalid_targets() {
        let conn = conn();
        let a = create_project(&conn, "A").unwrap().id.unwrap();
        let b = create_project(&conn, "B").unwrap().id.unwrap();
        let module_a = create_module(&conn, a, "认证").unwrap();
        let module_b = create_module(&conn, b, "计费").unwrap();
        let submodule_a = seed_submodule(&conn, module_a.id, "token 刷新");
        let submodule_b = seed_submodule(&conn, module_b.id, "账单");

        let task = create_task(
            &conn,
            Container::Project { id: a },
            Some(module_a.id),
            None,
            None,
            "登录限流",
        )
        .unwrap()
        .task;

        let attempts = [
            // 跨项目的模块
            MoveTarget {
                container: Container::Project { id: a },
                module_id: Some(module_b.id),
                submodule_id: None,
            },
            // 子模块缺模块
            MoveTarget {
                container: Container::Project { id: a },
                module_id: None,
                submodule_id: Some(submodule_a),
            },
            // 张冠李戴的子模块
            MoveTarget {
                container: Container::Project { id: a },
                module_id: Some(module_a.id),
                submodule_id: Some(submodule_b),
            },
            // 「我的待办」带模块
            MoveTarget {
                container: Container::Todo,
                module_id: Some(module_a.id),
                submodule_id: None,
            },
            // 未知项目
            MoveTarget {
                container: Container::Project { id: 999 },
                module_id: None,
                submodule_id: None,
            },
        ];

        for target in attempts {
            assert!(
                move_task(&conn, task.id, target).is_err(),
                "{target:?} 应被拒"
            );
            let after = load_task(&conn, task.id).unwrap();
            assert_eq!(after.project_id, Some(a));
            assert_eq!(after.module_id, Some(module_a.id));
            assert_eq!(after.submodule_id, None);
        }

        assert!(
            move_task(
                &conn,
                999,
                MoveTarget {
                    container: Container::Todo,
                    module_id: None,
                    submodule_id: None,
                }
            )
            .is_err()
        );
    }

    /// 子任务不能单独移动：归属的真相在父任务上。
    #[test]
    fn move_task_rejects_subtask() {
        let conn = conn();
        let target_project = create_project(&conn, "B").unwrap().id.unwrap();
        let parent = seed_task(&conn);
        let child = insert_child(&conn, &parent, "子任务", Status::Todo);

        let error = move_task(
            &conn,
            child,
            MoveTarget {
                container: Container::Project { id: target_project },
                module_id: None,
                submodule_id: None,
            },
        )
        .unwrap_err();

        assert!(error.contains("子任务"), "错误要说清原因：{error}");
        assert_eq!(load_task(&conn, parent.id).unwrap().project_id, None);
        assert_eq!(load_task(&conn, child).unwrap().project_id, None);
    }

    #[test]
    fn move_target_arg_deserializes_from_frontend() {
        let target: MoveTarget = serde_json::from_str(
            r#"{"container":{"kind":"project","id":3},"moduleId":7,"submoduleId":null}"#,
        )
        .unwrap();

        assert_eq!(target.module_id, Some(7));
        assert_eq!(target.submodule_id, None);
        assert!(matches!(target.container, Container::Project { id: 3 }));
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
            "createdAt",
            "startAt",
            "endAt",
            "sortOrder",
        ] {
            assert!(json.get(key).is_some(), "缺少 {key}");
        }
        assert_eq!(json["status"], "todo");
        assert_eq!(json["priority"], "mid");
        assert!(json["startAt"].is_null(), "未设置的时间是 null，不是缺字段");
        assert!(!json["createdAt"].is_null(), "新建任务应有创建时间");
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
    fn task_created_wire_shape_is_camel_case() {
        let conn = conn();
        let json = serde_json::to_value(TaskCreated {
            task: seed_task(&conn),
            notices: vec!["父任务已更新".to_string()],
        })
        .unwrap();

        assert_eq!(json["notices"][0], "父任务已更新");
        assert!(json["task"].get("parentTaskId").is_some());
        assert!(
            json["task"].get("parent_task_id").is_none(),
            "不能是 snake_case"
        );
    }

    #[test]
    fn delete_outcome_wire_shape_is_camel_case() {
        let json = serde_json::to_value(DeleteOutcome {
            notices: vec!["父任务已更新".to_string()],
        })
        .unwrap();
        assert_eq!(json["notices"][0], "父任务已更新");
    }

    #[test]
    fn status_outcome_wire_shape_is_camel_case() {
        let json = serde_json::to_value(StatusOutcome {
            changed: Vec::new(),
            notices: Vec::new(),
        })
        .unwrap();
        assert!(json.get("changed").is_some());
        assert!(json.get("notices").is_some());
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
            serde_json::from_str::<Priority>(r#""high""#)
                .unwrap()
                .as_str(),
            "high"
        );
        assert!(serde_json::from_str::<Status>(r#""blocked""#).is_err());
    }
}
