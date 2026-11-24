//! Domain layer: the only write entry point, and the only place allowed to express
//! "rules".
//!
//! The command layer only unpacks arguments and forwards them; SQL does not restate these
//! rules (no triggers), otherwise the same rule would have two implementations and
//! inevitably drift.

use chrono::Local;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::message::{Message, MessageKey, Res};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Task status. Three states; clicking the cell cycles through these values.
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
            other => Err(Message::new(MessageKey::UnknownStatus).param("value", other)),
        }
    }
}

/// Task priority. Defaults to `Mid`.
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
            other => Err(Message::new(MessageKey::UnknownPriority).param("value", other)),
        }
    }

    /// A larger number means higher priority. "A subtask cannot outrank its parent"
    /// compares only this value.
    fn rank(self) -> u8 {
        match self {
            Priority::High => 3,
            Priority::Mid => 2,
            Priority::Low => 1,
        }
    }

    /// Priorities strictly above `self`, used to cascade subtasks down when a parent is
    /// lowered.
    fn above(self) -> &'static [Priority] {
        match self {
            Priority::High => &[],
            Priority::Mid => &[Priority::High],
            Priority::Low => &[Priority::High, Priority::Mid],
        }
    }
}

/// The container a task lives in: the virtual "My Todos", or a real project.
///
/// Serialized as `{ kind: "todo" }` / `{ kind: "project", id }`; the frontend sends it
/// back verbatim.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Container {
    Todo,
    Project { id: i64 },
}

impl Container {
    /// The corresponding `task.project_id` (NULL means "My Todos").
    fn project_id(self) -> Option<i64> {
        match self {
            Container::Todo => None,
            Container::Project { id } => Some(id),
        }
    }
}

/// Destination of `move_task`: container plus two classification levels (both optional).
///
/// Same shape as the placement arguments of `create_task`, sent back verbatim by the
/// frontend; validity rules are shared through `require_task_target`, so moving does not
/// get a second set of checks.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveTarget {
    pub container: Container,
    pub module_id: Option<i64>,
    pub submodule_id: Option<i64>,
}

/// A two-level classification node that can be deleted as a whole group.
///
/// Module and submodule ids are independent (both start at 1), so a single id cannot
/// tell them apart; commands must carry `kind` as well.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Module,
    Submodule,
}

/// One sidebar entry: a project, or the virtual "My Todos" (`id` is `None`, `is_todo`
/// is `true`).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: Option<i64>,
    pub name: String,
    pub is_todo: bool,
    /// Number of top-level tasks. Subtasks are not counted—the sidebar is navigation,
    /// not a progress dashboard.
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
    /// Local time `YYYY-MM-DDTHH:MM`; written when the task is created, `None` for tasks
    /// from before the upgrade.
    pub created_at: Option<String>,
    /// Local time `YYYY-MM-DDTHH:MM`, `None` = not recorded; maintained automatically by
    /// status changes.
    pub start_at: Option<String>,
    pub end_at: Option<String>,
    pub sort_order: i64,
}

/// Return of `plan_set_status`: what must be told to the frontend before changing this
/// status.
///
/// The whole point of the two-phase flow is `needs_confirm`: the frontend does not
/// recompute rules, it only decides whether to show a dialog based on this flag.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusPlan {
    /// Affected tasks (the target task plus subtasks that must change with it). Used to
    /// report the count.
    pub affects: Vec<Task>,
    pub needs_confirm: bool,
    /// Structured reason shown only when `needs_confirm` is true.
    pub reason: Option<Message>,
}

/// A parent status change proposed by aggregation. The parent has not been written yet;
/// the frontend asks the user, then applies it by setting the parent's status.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParentStatusProposal {
    pub parent_task_id: i64,
    pub parent_title: String,
    pub status: Status,
    pub message: Message,
}

/// Return of `apply_set_status`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusOutcome {
    pub changed: Vec<Task>,
    /// Toast shown when the parent task is changed automatically. The user's own click
    /// does not produce a notice.
    pub notices: Vec<Message>,
    /// Parent completions that still need the user's confirmation.
    pub parent_proposals: Vec<ParentStatusProposal>,
}

/// Pre-delete notice: how many tasks (including subtasks) this will remove.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletePlan {
    pub task_count: i64,
}

/// Return of `create_task`: the task itself plus toasts for parent tasks changed by
/// aggregation.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskCreated {
    pub task: Task,
    /// When creating a subtask pushes a completed parent back to doing, there is a
    /// notice here.
    pub notices: Vec<Message>,
}

/// Return of `delete_task`: toasts when deleting a subtask changes its parent
/// automatically.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteOutcome {
    pub notices: Vec<Message>,
    /// Parent completions that still need the user's confirmation.
    pub parent_proposals: Vec<ParentStatusProposal>,
}

/// Return of `reparent_task`: the task after the hierarchy change plus toasts for parent
/// tasks changed by aggregation.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReparentOutcome {
    pub task: Task,
    pub notices: Vec<Message>,
    /// Parent completions that still need the user's confirmation.
    pub parent_proposals: Vec<ParentStatusProposal>,
}

/// Column order of the task table; this is the only copy used by task readers, avoiding
/// `row.get(i)` index drift.
const TASK_COLUMNS: &str = "id, project_id, module_id, submodule_id, parent_task_id, \
                            title, status, priority, note, sort_order, start_at, end_at, created_at";

// ---------------------------------------------------------------------------
// Read
// ---------------------------------------------------------------------------

/// All sidebar entries: the first is always "My Todos", the rest are ordered by
/// `sort_order`.
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
        // The todo container has no user-defined name; the frontend renders
        // its label from `is_todo`.
        name: String::new(),
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

/// A project's module tree (modules plus their submodules). "My Todos" has no modules,
/// so this command is not called for it.
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

/// All tasks in a container (including subtasks; grouping and sorting happen on the
/// frontend).
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
// Write
// ---------------------------------------------------------------------------

pub fn create_project(conn: &Connection, name: &str) -> Res<Project> {
    let name = require_name(name, "project")?;
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
    let name = require_name(name, "module")?;
    require_project(conn, project_id)?;
    ensure_module_name_free(conn, project_id, name, None)?;
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

/// Create a submodule. Classification is at most two levels deep, so nothing can hang
/// below a submodule.
pub fn create_submodule(conn: &Connection, module_id: i64, name: &str) -> Res<Submodule> {
    let name = require_name(name, "submodule")?;
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

/// Reorder modules within a project: insert `id` before `before_id`; `None` moves it to
/// the end.
///
/// Only modules in the same project can be reordered against each other; both ids must
/// exist. Afterwards the whole project's `sort_order` is rewritten as `0..n-1`, which also
/// repairs any duplicate order values left in historical data. Submodules follow their
/// module and do not take part in this ordering.
pub fn move_module(conn: &Connection, id: i64, before_id: Option<i64>) -> Res<Vec<Module>> {
    let project_id: i64 = conn
        .query_row(
            "SELECT project_id FROM module WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(|err| match err {
            rusqlite::Error::QueryReturnedNoRows => not_found("module", id),
            other => db_err(other),
        })?;

    // Dropping before itself is a no-op; return the current order without rewriting
    // every sort order for nothing.
    if before_id == Some(id) {
        return list_modules(conn, project_id);
    }

    let tx = conn.unchecked_transaction().map_err(db_err)?;

    if let Some(target) = before_id {
        let target_project: i64 = tx
            .query_row(
                "SELECT project_id FROM module WHERE id = ?1",
                params![target],
                |row| row.get(0),
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => not_found("module", target),
                other => db_err(other),
            })?;
        if target_project != project_id {
            return Err(Message::new(MessageKey::ModuleOrderSameProject));
        }
    }

    let ids: Vec<i64> = {
        let mut stmt = tx
            .prepare("SELECT id FROM module WHERE project_id = ?1 ORDER BY sort_order, id")
            .map_err(db_err)?;
        stmt.query_map(params![project_id], |row| row.get(0))
            .map_err(db_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_err)?
    };

    let mut reordered: Vec<i64> = ids.iter().copied().filter(|&other| other != id).collect();
    let insert_at = match before_id {
        Some(target) => reordered
            .iter()
            .position(|&other| other == target)
            .ok_or_else(|| not_found("module", target))?,
        None => reordered.len(),
    };
    reordered.insert(insert_at, id);

    if reordered != ids {
        for (index, module_id) in reordered.iter().enumerate() {
            tx.execute(
                "UPDATE module SET sort_order = ?1 WHERE id = ?2",
                params![index as i64, module_id],
            )
            .map_err(db_err)?;
        }
    }

    tx.commit().map_err(db_err)?;
    list_modules(conn, project_id)
}

pub fn rename_project(conn: &Connection, id: i64, name: &str) -> Res<Project> {
    rename_row(conn, "project", id, name, "project")?;
    load_project(conn, id)
}

pub fn rename_module(conn: &Connection, id: i64, name: &str) -> Res<Module> {
    let name = require_name(name, "module")?;
    let project_id: i64 = conn
        .query_row(
            "SELECT project_id FROM module WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(|err| match err {
            rusqlite::Error::QueryReturnedNoRows => not_found("module", id),
            other => db_err(other),
        })?;
    ensure_module_name_free(conn, project_id, name, Some(id))?;
    rename_row(conn, "module", id, name, "module")?;
    load_module(conn, id)
}

pub fn rename_submodule(conn: &Connection, id: i64, name: &str) -> Res<Submodule> {
    rename_row(conn, "submodule", id, name, "submodule")?;
    load_submodule(conn, id)
}

/// Create a task. When `parent_task_id` is `Some`, create one level of subtask.
///
/// Top-level placement rules: the container determines the project; `module_id` /
/// `submodule_id` come from the caller based on the "current scope", and leaving them out
/// means "default". Priority defaults to mid, and the sort order is the maximum within
/// **the same group** + 1 (last in the group).
///
/// A subtask's true placement lives on its parent: the container must match the parent's,
/// `module_id` / `submodule_id` are always NULL (ignored even if passed), and sort order is
/// queued separately among the parent's subtasks. Only one level is allowed—reject when the
/// parent is itself a subtask. Priority cannot exceed the parent's: when the parent is low,
/// the new subtask is low too.
pub fn create_task(
    conn: &Connection,
    container: Container,
    module_id: Option<i64>,
    submodule_id: Option<i64>,
    parent_task_id: Option<i64>,
    title: &str,
) -> Res<TaskCreated> {
    let title = require_name(title, "task")?;

    // Validate the parent before opening a transaction: a validation failure leaves no
    // half-open transaction, and the code reads more directly.
    let (parent_project, parent_priority) = match parent_task_id {
        None => (None, None),
        Some(parent_id) => {
            let parent = load_task(conn, parent_id)?;
            if parent.parent_task_id.is_some() {
                return Err(Message::new(MessageKey::SubtaskCannotNest));
            }
            if parent.project_id != container.project_id() {
                return Err(Message::new(MessageKey::SubtaskContainerMismatch));
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
        // Subtasks do not store the two classification levels: the parent expresses
        // placement, and a redundant copy would only drift.
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

    // Default to mid, but follow a lower-priority parent so a new subtask is never
    // invalid from birth.
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
            // Defaults are written by the domain layer, not by the schema DEFAULT:
            // there is only one source of truth for rules.
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

    // Creating a subtask is also an aggregation event: a completed parent must go back
    // to doing.
    let mut notices = Vec::new();
    if let Some(parent_id) = parent_task_id {
        // A newly created subtask is always todo, so it can only reopen a parent,
        // never complete one.
        let (parent_notices, _) = split_parent_change(aggregate_parent_status(&tx, parent_id)?);
        notices.extend(parent_notices);
    }

    tx.commit().map_err(db_err)?;

    Ok(TaskCreated { task, notices })
}

/// Rename in place. Reject if empty after trimming—the frontend restores the original
/// text, and not a single byte in the database changes.
pub fn rename_task(conn: &Connection, id: i64, title: &str) -> Res<Task> {
    let title = require_name(title, "task")?;
    let mut task = load_task(conn, id)?;

    conn.execute(
        "UPDATE task SET title = ?1 WHERE id = ?2",
        params![title, id],
    )
    .map_err(db_err)?;

    task.title = title.to_string();
    Ok(task)
}

/// Change priority. No confirmation involved: reordering within the group is the cost of
/// this field, so there is nothing to ask.
///
/// A subtask cannot outrank its parent: the menu never offers an invalid option, but this
/// rejects one as a backstop. When a parent is lowered, subtasks still above it are
/// lowered in the same transaction.
pub fn set_priority(conn: &Connection, id: i64, priority: Priority) -> Res<Task> {
    let mut task = load_task(conn, id)?;

    if let Some(parent_id) = task.parent_task_id {
        let parent = load_task(conn, parent_id)?;
        if priority.rank() > parent.priority.rank() {
            return Err(Message::new(MessageKey::SubtaskPriorityAboveParent));
        }
    }

    let tx = conn.unchecked_transaction().map_err(db_err)?;
    tx.execute(
        "UPDATE task SET priority = ?1 WHERE id = ?2",
        params![priority.as_str(), id],
    )
    .map_err(db_err)?;

    // Lowering hits at most two old levels; one UPDATE per level reads better than
    // repeating the level ordering in SQL.
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

/// Change the note. **No trimming, no empty rejection**: a note may be any text
/// (including newlines and leading/trailing whitespace), and an empty string means clear
/// it. The single-line summary in the table is a presentation concern; the stored value
/// stays as-is.
pub fn set_note(conn: &Connection, id: i64, note: &str) -> Res<Task> {
    let mut task = load_task(conn, id)?;

    conn.execute("UPDATE task SET note = ?1 WHERE id = ?2", params![note, id])
        .map_err(db_err)?;

    task.note = note.to_string();
    Ok(task)
}

/// Move a top-level task to a different classification: container, module, and submodule
/// can all change together.
///
/// Target-group validity is fully delegated to `require_task_target` (the same rules as
/// creation); after moving, the task goes to the end of the target group. Subtasks cannot
/// be moved on their own—their true placement lives on the parent—so here only their
/// `project_id` is switched to the new container.
pub fn move_task(conn: &Connection, id: i64, target: MoveTarget) -> Res<Task> {
    let mut task = load_task(conn, id)?;
    if task.parent_task_id.is_some() {
        return Err(Message::new(MessageKey::SubtaskCannotMove));
    }

    let project_id = require_task_target(
        conn,
        target.container,
        target.module_id,
        target.submodule_id,
    )?;
    let tx = conn.unchecked_transaction().map_err(db_err)?;

    // The exact same grouping condition as `create_task`: the new sort order is that
    // group's MAX + 1.
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

    // Subtasks follow the container; the two classification levels stay NULL because
    // the parent still expresses placement.
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

/// Move a task under a parent: `Some(target)` makes it a subtask of that target (a
/// top-level task is demoted; a subtask is retargeted), `None` promotes it to a top-level
/// task.
///
/// Only one parent-child level is allowed. The target must be a task in the same container
/// with no parent, and the moved task cannot bring its own subtasks along—otherwise
/// grandchild tasks would appear. Promotion inherits the old parent's classification and
/// appends to the end of that top-level group. Every affected parent is re-aggregated: a
/// retarget touches both the old and the new parent.
pub fn reparent_task(
    conn: &Connection,
    id: i64,
    parent_task_id: Option<i64>,
) -> Res<ReparentOutcome> {
    let task = load_task(conn, id)?;
    let previous_parent_id = task.parent_task_id;

    // Moving a subtask to its current parent is a no-op (also avoids aggregating it twice).
    if parent_task_id.is_some() && parent_task_id == previous_parent_id {
        return Ok(ReparentOutcome {
            task,
            notices: Vec::new(),
            parent_proposals: Vec::new(),
        });
    }

    let tx = conn.unchecked_transaction().map_err(db_err)?;

    let (task, notices, parent_proposals) = match parent_task_id {
        Some(target_id) => {
            if !load_children(&tx, id)?.is_empty() {
                return Err(Message::new(MessageKey::HasChildrenCannotDemote));
            }
            if target_id == id {
                return Err(Message::new(MessageKey::CannotDemoteToSelf));
            }

            let target = load_task(&tx, target_id)?;
            if target.parent_task_id.is_some() {
                return Err(Message::new(MessageKey::SubtaskCannotNest));
            }
            if target.project_id != task.project_id {
                return Err(Message::new(MessageKey::SubtaskParentContainerMismatch));
            }

            // A subtask cannot outrank its parent, or the sort rule would push it above
            // the parent.
            let priority = if task.priority.rank() > target.priority.rank() {
                target.priority
            } else {
                task.priority
            };
            let sort_order =
                next_sort_order(&tx, "task", "parent_task_id = ?1", params![target_id])?;

            tx.execute(
                "UPDATE task SET parent_task_id = ?1, project_id = ?2, module_id = NULL, \
                 submodule_id = NULL, priority = ?3, sort_order = ?4 WHERE id = ?5",
                params![
                    target_id,
                    target.project_id,
                    priority.as_str(),
                    sort_order,
                    id
                ],
            )
            .map_err(db_err)?;

            let updated = load_task(&tx, id)?;
            let mut notices = Vec::new();
            let mut parent_proposals = Vec::new();

            // Retargeting a subtask changes two parents: the one it left and the one it
            // joined. Both must be re-aggregated.
            if let Some(old_parent_id) = previous_parent_id {
                let (old_notices, old_proposals) =
                    split_parent_change(aggregate_parent_status(&tx, old_parent_id)?);
                notices.extend(old_notices);
                parent_proposals.extend(old_proposals);
            }
            let (new_notices, new_proposals) =
                split_parent_change(aggregate_parent_status(&tx, target_id)?);
            notices.extend(new_notices);
            parent_proposals.extend(new_proposals);

            (updated, notices, parent_proposals)
        }
        None => {
            let parent_id = task
                .parent_task_id
                .ok_or(Message::new(MessageKey::AlreadyTopLevel))?;
            let parent = load_task(&tx, parent_id)?;
            let sort_order = next_sort_order(
                &tx,
                "task",
                "project_id IS ?1 AND module_id IS ?2 AND submodule_id IS ?3 \
                 AND parent_task_id IS NULL",
                params![parent.project_id, parent.module_id, parent.submodule_id],
            )?;

            tx.execute(
                "UPDATE task SET parent_task_id = NULL, project_id = ?1, module_id = ?2, \
                 submodule_id = ?3, sort_order = ?4 WHERE id = ?5",
                params![
                    parent.project_id,
                    parent.module_id,
                    parent.submodule_id,
                    sort_order,
                    id
                ],
            )
            .map_err(db_err)?;

            let updated = load_task(&tx, id)?;
            let (notices, parent_proposals) =
                split_parent_change(aggregate_parent_status(&tx, parent_id)?);
            (updated, notices, parent_proposals)
        }
    };

    tx.commit().map_err(db_err)?;
    Ok(ReparentOutcome {
        task,
        notices,
        parent_proposals,
    })
}

/// First step of changing status: only compute "is confirmation needed", without writing.
///
/// There is only one case that needs confirmation: marking a task as done while it still
/// has unfinished subtasks. Reverting (to todo / doing) never touches subtasks, so it
/// needs no confirmation.
pub fn plan_set_status(conn: &Connection, id: i64, status: Status) -> Res<StatusPlan> {
    let task = load_task(conn, id)?;
    let open_children = if status == Status::Done {
        unfinished_children(conn, id)?
    } else {
        Vec::new()
    };

    let needs_confirm = !open_children.is_empty();
    let reason = needs_confirm.then(|| {
        Message::new(MessageKey::CascadeDone).param("count", open_children.len())
    });

    let mut affects = vec![task];
    affects.extend(open_children);

    Ok(StatusPlan {
        affects,
        needs_confirm,
        reason,
    })
}

/// Second step of changing status: write to the database. When `cascade` is true, mark
/// unfinished subtasks as done as well.
///
/// If the target is itself a subtask, parent aggregation runs after the write (all children
/// done → parent done, otherwise if the parent is done → parent goes back to doing). When
/// the target is a parent task (including cascade writes to subtasks), no upward
/// aggregation happens; manually changing a parent never touches its subtasks.
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

    // Aggregate only when the target itself is a subtask; a manual change to a parent
    // produces no notice.
    let mut parent_proposals = Vec::new();
    if let Some(parent_id) = parent_id {
        match aggregate_parent_status(&tx, parent_id)? {
            Some(ParentChange::Applied { task, notice }) => {
                changed.push(task);
                notices.push(notice);
            }
            Some(ParentChange::Proposed(proposal)) => parent_proposals.push(proposal),
            None => {}
        }
    }

    tx.commit().map_err(db_err)?;

    Ok(StatusOutcome {
        changed,
        notices,
        parent_proposals,
    })
}

// ---------------------------------------------------------------------------
// Delete
// ---------------------------------------------------------------------------

/// First step of deleting a task: only count how many rows will be removed, without
/// writing.
///
/// Subtasks are removed through the `parent_task_id` cascade, so the count must include
/// them; the frontend tells the user this number directly.
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

/// Delete a task. Deletion and parent aggregation run in the same transaction: removing
/// the last subtask makes the parent done under the aggregation rules and returns a toast.
/// Deleting a parent only cascades downward; no upward aggregation.
pub fn delete_task(conn: &Connection, id: i64) -> Res<DeleteOutcome> {
    // Remember placement first: after DELETE the parent id can no longer be read.
    let task = load_task(conn, id)?;
    let parent_id = task.parent_task_id;
    let tx = conn.unchecked_transaction().map_err(db_err)?;

    let removed = tx
        .execute("DELETE FROM task WHERE id = ?1", params![id])
        .map_err(db_err)?;
    if removed == 0 {
        return Err(not_found("task", id));
    }

    let mut notices = Vec::new();
    let mut parent_proposals = Vec::new();
    if let Some(parent_id) = parent_id {
        let (parent_notices, proposals) =
            split_parent_change(aggregate_parent_status(&tx, parent_id)?);
        notices.extend(parent_notices);
        parent_proposals.extend(proposals);
    }

    tx.commit().map_err(db_err)?;
    Ok(DeleteOutcome {
        notices,
        parent_proposals,
    })
}

/// First step of deleting a module / submodule: only count how many rows will be removed,
/// without writing.
///
/// For a module, the count must also include tasks in its submodules—they are removed by
/// the same cascade.
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
    let (table, entity) = match kind {
        NodeKind::Module => ("module", "module"),
        NodeKind::Submodule => ("submodule", "submodule"),
    };
    let removed = conn
        .execute(&format!("DELETE FROM {table} WHERE id = ?1"), params![id])
        .map_err(db_err)?;
    if removed == 0 {
        return Err(not_found(entity, id));
    }
    Ok(())
}

/// First step of deleting a project: every task in the project is removed by the cascade.
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
        return Err(not_found("project", id));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Write status and maintain start / end times according to the status change.
///
/// Times move only when the status **really changes**; rewriting the same status does not
/// overwrite existing times:
/// - entering `doing`: record the current time if start is empty; clear end (still working,
///   not finished yet)
/// - entering `done`: record the current time as end
/// - back to `todo`: clear both (this attempt is void; the next start records anew)
///
/// Aggregation-driven parent changes also go through here, so automatic completion /
/// restoration maintains times in the same way.
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

/// Local time recorded on status changes, accurate to the minute; same format as
/// `start_at` / `end_at`.
fn now_local() -> String {
    Local::now().format("%Y-%m-%dT%H:%M").to_string()
}

fn load_task(conn: &Connection, id: i64) -> Res<Task> {
    let sql = format!("SELECT {TASK_COLUMNS} FROM task WHERE id = ?1");
    conn.query_row(&sql, params![id], map_task)
        .map_err(|err| match err {
            rusqlite::Error::QueryReturnedNoRows => not_found("task", id),
            other => db_err(other),
        })?
        .into_task()
}

/// Unfinished subtasks of a task (one level, no recursion: a subtask cannot have its own
/// subtasks).
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

/// All subtasks of a parent task (one level, no recursion).
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

/// Result of a parent task changed by aggregation: the updated task plus one sentence for
/// the user.
#[derive(Debug)]
enum ParentChange {
    /// The parent status was written; callers add it to the result and show the notice.
    Applied { task: Task, notice: Message },
    /// Completing a parent needs the user's confirmation; nothing was written yet.
    Proposed(ParentStatusProposal),
}

fn split_parent_change(
    change: Option<ParentChange>,
) -> (Vec<Message>, Vec<ParentStatusProposal>) {
    match change {
        Some(ParentChange::Applied { notice, .. }) => (vec![notice], Vec::new()),
        Some(ParentChange::Proposed(proposal)) => (Vec::new(), vec![proposal]),
        None => (Vec::new(), Vec::new()),
    }
}

/// Event-driven parent status derivation; returns `Some` only when the parent status is
/// **actually rewritten**.
///
/// The rule is implemented literally: all subtasks `done` → parent `done`; otherwise if
/// any subtask is `doing` or `done` → parent `doing`; all subtasks `todo` → parent `todo`.
/// With zero subtasks `all()` is true, so deleting the last unfinished subtask also pushes
/// the parent to done—this is the specified derivation, not an invariant.
fn aggregate_parent_status(conn: &Connection, parent_id: i64) -> Res<Option<ParentChange>> {
    let mut parent = load_task(conn, parent_id)?;
    let children = load_children(conn, parent_id)?;
    let all_done = children.iter().all(|child| child.status == Status::Done);
    let any_started = children.iter().any(|child| child.status != Status::Todo);
    let was_done = parent.status == Status::Done;

    let next = if all_done {
        if was_done {
            return Ok(None);
        }
        Status::Done
    } else if any_started {
        if parent.status == Status::Doing {
            return Ok(None);
        }
        Status::Doing
    } else {
        if parent.status == Status::Todo {
            return Ok(None);
        }
        Status::Todo
    };

    // Completing the parent is the one aggregation result that asks first. Leave the
    // parent untouched and propose the change; the frontend confirms, then writes it.
    if next == Status::Done {
        let message = Message::new(MessageKey::ConfirmParentAutoDone)
            .param("title", parent.title.clone());
        return Ok(Some(ParentChange::Proposed(ParentStatusProposal {
            parent_task_id: parent.id,
            parent_title: parent.title,
            status: Status::Done,
            message,
        })));
    }

    write_status(conn, &mut parent, next, &now_local())?;
    let notice = Message::new(match next {
        Status::Done => MessageKey::ParentAutoDone,
        Status::Doing if was_done => MessageKey::ParentRestoredDoing,
        Status::Doing => MessageKey::ParentAutoDoing,
        Status::Todo if was_done => MessageKey::ParentRestoredTodo,
        Status::Todo => MessageKey::ParentAutoTodo,
    });

    Ok(Some(ParentChange::Applied {
        task: parent,
        notice,
    }))
}
/// Row → task. `status` / `priority` are text columns; parsing failures must blow up at
/// this layer.
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

/// Name-like fields are always trimmed; reject if empty after trimming (the frontend
/// checks too, but the backend must not trust it alone).
fn require_name<'a>(raw: &'a str, field: &'static str) -> Res<&'a str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Message::new(MessageKey::NameRequired).param("field", field));
    }
    Ok(trimmed)
}

/// Module names are unique within a project after trimming. `except_id` lets a rename
/// keep its current name.
fn ensure_module_name_free(
    conn: &Connection,
    project_id: i64,
    name: &str,
    except_id: Option<i64>,
) -> Res<()> {
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM module WHERE project_id = ?1 AND name = ?2 \
             AND (?3 IS NULL OR id != ?3))",
            params![project_id, name, except_id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if exists {
        return Err(Message::new(MessageKey::ModuleNameDuplicate).param("name", name));
    }
    Ok(())
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
        Err(not_found("project", id))
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
        Err(not_found("module", module_id))
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
        Err(not_found("submodule", submodule_id))
    }
}

/// Update a structure row that only has a `name`. `table` / `label` / `field` accept only
/// literals from this file.
fn rename_row(
    conn: &Connection,
    table: &str,
    id: i64,
    name: &str,
    entity: &'static str,
) -> Res<()> {
    let name = require_name(name, entity)?;
    let changed = conn
        .execute(
            &format!("UPDATE {table} SET name = ?1 WHERE id = ?2"),
            params![name, id],
        )
        .map_err(db_err)?;

    if changed == 0 {
        return Err(not_found(entity, id));
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
        rusqlite::Error::QueryReturnedNoRows => not_found("project", id),
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
            rusqlite::Error::QueryReturnedNoRows => not_found("module", id),
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
        rusqlite::Error::QueryReturnedNoRows => not_found("submodule", id),
        other => db_err(other),
    })
}

/// A module must really belong to this project: a module from project A cannot be attached
/// to a task in project B.
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
        Err(Message::new(MessageKey::ModuleNotInProject)
            .param("module", module_id)
            .param("project", project_id))
    }
}

/// A submodule must really belong to this module.
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
        Err(Message::new(MessageKey::SubmoduleNotInModule)
            .param("submodule", submodule_id)
            .param("module", module_id))
    }
}

/// The single validation of task placement: both creation and moving go through here.
///
/// Returns the `project_id` the task should land in (`None` for "My Todos"). Rules:
/// when the container is a project, the project must exist; when a module is given, it
/// must belong to that project; "My Todos" cannot have a module; when a submodule is
/// given, a module must come first and the submodule must belong to it.
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
        // "My Todos" has no modules; passing a module is the caller's mistake.
        (None, Some(_)) => return Err(Message::new(MessageKey::TodoHasNoModules)),
        (Some(project_id), None) => require_project(conn, project_id)?,
        (None, None) => {}
    }
    if let Some(submodule_id) = submodule_id {
        let module_id = module_id.ok_or(Message::new(MessageKey::SubmoduleNeedsModule))?;
        require_submodule(conn, module_id, submodule_id)?;
    }
    Ok(project_id)
}

/// Next sort order within a group (0 when the group has no tasks).
///
/// `where_clause` may only come from literals in this file—it is a SQL fragment and must
/// never accept external input.
fn next_sort_order(
    conn: &Connection,
    table: &str,
    where_clause: &str,
    args: impl rusqlite::Params,
) -> Res<i64> {
    let sql = format!("SELECT COALESCE(MAX(sort_order), -1) + 1 FROM {table} WHERE {where_clause}");
    conn.query_row(&sql, args, |row| row.get(0)).map_err(db_err)
}

fn db_err(err: rusqlite::Error) -> Message {
    Message::new(MessageKey::DbOperationFailed).param("detail", err.to_string())
}

fn not_found(entity: &'static str, id: i64) -> Message {
    Message::new(MessageKey::NotFound)
        .param("entity", entity)
        .param("id", id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::MessageParam;

    fn assert_error_key<T: std::fmt::Debug>(result: Res<T>, expected: MessageKey) {
        let error = result.unwrap_err();
        assert_eq!(error.key, expected, "unexpected error: {error}");
    }

    fn assert_not_found<T: std::fmt::Debug>(result: Res<T>, entity: &str, id: i64) {
        let error = result.unwrap_err();
        assert_eq!(error.key, MessageKey::NotFound, "unexpected error: {error}");
        assert_eq!(
            error.params.get("entity"),
            Some(&MessageParam::Text(entity.to_owned()))
        );
        assert_eq!(error.params.get("id"), Some(&MessageParam::Number(id)));
    }

    /// Simulates the frontend accepting a proposed parent completion.
    fn confirm_proposal(conn: &Connection, proposal: &ParentStatusProposal) {
        apply_set_status(conn, proposal.parent_task_id, proposal.status, false).unwrap();
    }

    /// In-memory database with migrations applied: domain tests need no window and no
    /// real file.
    fn conn() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        crate::migrations::migrate(&mut conn).unwrap();
        conn
    }

    /// Smallest vertical slice of the main flow: create project → create task → read it
    /// back.
    #[test]
    fn new_task_lands_unclassified_in_project() {
        let conn = conn();
        let project = create_project(&conn, "knot-tr").unwrap();
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
        assert_eq!(task.module_id, None, "a new task lands in the default group");
        assert_eq!(task.submodule_id, None);
        assert_eq!(task.priority.as_str(), "mid", "the default priority is mid");
        assert_eq!(task.status.as_str(), "todo");
        assert_eq!(task.sort_order, 0);

        let tasks = list_tasks(&conn, Container::Project { id: project_id }).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "处理并发刷新");
    }

    /// "My Todos" is not a project row: it is expressed by `project_id IS NULL` and never
    /// appears in the project list.
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
        assert!(projects[0].is_todo, "the first entry is always \"My Todos\"");
        assert_eq!(projects[0].id, None);
        assert_eq!(projects[0].task_count, 1);

        let real = projects.iter().find(|p| p.id == project.id).unwrap();
        assert_eq!(real.task_count, 0, "tasks in My Todos do not count toward a project");

        let tasks = list_tasks(&conn, Container::Todo).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].project_id, None);
    }

    /// Sort order increases within a group: a new task goes last in the group.
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
        // Different containers order independently
        assert_eq!(
            create_task(&conn, Container::Todo, None, None, None, "c")
                .unwrap()
                .task
                .sort_order,
            0
        );
    }

    /// Blank text is blocked on the frontend, but the backend must block it too—never
    /// trust the frontend alone.
    #[test]
    fn rejects_blank_names() {
        let conn = conn();
        assert!(create_project(&conn, "   ").is_err());
        assert!(create_task(&conn, Container::Todo, None, None, None, "\t\n").is_err());
    }

    /// Whitespace around names is trimmed; the clean value lands in the database.
    #[test]
    fn trims_names() {
        let conn = conn();
        let project = create_project(&conn, "  认证  ").unwrap();
        assert_eq!(project.name, "认证");
    }

    /// Module names are unique within one project, after trimming; other projects and
    /// renaming a module to its own current name are fine.
    #[test]
    fn module_names_are_unique_within_a_project() {
        let conn = conn();
        let project_id = create_project(&conn, "knot-tr").unwrap().id.unwrap();
        create_module(&conn, project_id, "认证").unwrap();

        let error = create_module(&conn, project_id, "认证").unwrap_err();
        assert_eq!(error.key, MessageKey::ModuleNameDuplicate);
        assert_eq!(
            error.params.get("name"),
            Some(&MessageParam::Text("认证".to_owned()))
        );

        let error = create_module(&conn, project_id, "  认证  ").unwrap_err();
        assert_eq!(
            error.params.get("name"),
            Some(&MessageParam::Text("认证".to_owned())),
            "the trimmed name is what gets compared"
        );

        let other_project = create_project(&conn, "别的项目").unwrap().id.unwrap();
        create_module(&conn, other_project, "认证").unwrap();

        let billing = create_module(&conn, project_id, "计费").unwrap();
        assert_error_key(
            rename_module(&conn, billing.id, "认证"),
            MessageKey::ModuleNameDuplicate,
        );
        rename_module(&conn, billing.id, "计费").unwrap();
        assert_eq!(
            rename_module(&conn, billing.id, "报表").unwrap().name,
            "报表"
        );
    }

    /// Creating a task under a nonexistent project must error, not leave a dangling
    /// foreign key.
    #[test]
    fn rejects_unknown_project() {
        let conn = conn();
        assert!(create_task(&conn, Container::Project { id: 999 }, None, None, None, "x").is_err());
        assert!(create_module(&conn, 999, "m").is_err());
    }

    /// The module tree is assembled by sort_order, with submodules hanging under their own
    /// module.
    #[test]
    fn modules_carry_their_submodules() {
        let conn = conn();
        let project = create_project(&conn, "knot-tr").unwrap();
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
    // New tasks land in the current scope
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
        let project = create_project(&conn, "knot-tr").unwrap();
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
        assert_eq!(task.submodule_id, None, "selecting only a module lands in its direct list");
    }

    #[test]
    fn task_created_in_submodule_lands_there() {
        let conn = conn();
        let project = create_project(&conn, "knot-tr").unwrap();
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

    /// Sort order is per **group**: tasks in a module and default tasks order separately.
    #[test]
    fn sort_order_is_per_group() {
        let conn = conn();
        let project = create_project(&conn, "knot-tr").unwrap();
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
            "another group starts counting from zero"
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

    /// Module and submodule placement must be real: no cross-project, no mismatched
    /// pairs.
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
        let project_id = create_project(&conn, "knot-tr").unwrap().id.unwrap();
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

    /// "My Todos" has no modules; passing a module is the caller's mistake, not something
    /// to silently treat as the default group.
    #[test]
    fn rejects_module_on_todo_container() {
        let conn = conn();
        let project_id = create_project(&conn, "knot-tr").unwrap().id.unwrap();
        let module = create_module(&conn, project_id, "认证").unwrap();

        assert!(create_task(&conn, Container::Todo, Some(module.id), None, None, "x").is_err());
    }

    // -----------------------------------------------------------------------
    // Structure: creating submodules and renaming all three levels
    // -----------------------------------------------------------------------

    #[test]
    fn create_submodule_appends_in_its_module() {
        let conn = conn();
        let project_id = create_project(&conn, "knot-tr").unwrap().id.unwrap();
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
        assert!(billing_now.submodules.is_empty(), "sort order is counted per module");
    }

    #[test]
    fn create_submodule_rejects_unknown_module() {
        let conn = conn();
        assert!(create_submodule(&conn, 999, "x").is_err());
    }

    #[test]
    fn create_submodule_rejects_blank_name() {
        let conn = conn();
        let project_id = create_project(&conn, "knot-tr").unwrap().id.unwrap();
        let module = create_module(&conn, project_id, "认证").unwrap();
        assert!(create_submodule(&conn, module.id, "   ").is_err());
    }

    /// Sidebar drag ordering: can insert before any module or move to the end; afterwards
    /// sort orders are contiguous `0..n-1`, and newly created modules still append at the
    /// end.
    #[test]
    fn moves_modules_within_project() {
        let conn = conn();
        let project_id = create_project(&conn, "knot-tr").unwrap().id.unwrap();
        let auth = create_module(&conn, project_id, "认证").unwrap();
        let billing = create_module(&conn, project_id, "计费").unwrap();
        let report = create_module(&conn, project_id, "报表").unwrap();

        let names = |modules: &[Module]| modules.iter().map(|m| m.name.clone()).collect::<Vec<_>>();

        // Move the last one to the front
        let modules = move_module(&conn, report.id, Some(auth.id)).unwrap();
        assert_eq!(names(&modules), vec!["报表", "认证", "计费"]);

        // Move to the middle: insert billing before auth
        let modules = move_module(&conn, billing.id, Some(auth.id)).unwrap();
        assert_eq!(names(&modules), vec!["报表", "计费", "认证"]);

        // None = move to the end
        let modules = move_module(&conn, report.id, None).unwrap();
        assert_eq!(names(&modules), vec!["计费", "认证", "报表"]);

        // Dropping before itself is a no-op and changes nothing
        let modules = move_module(&conn, billing.id, Some(billing.id)).unwrap();
        assert_eq!(names(&modules), vec!["计费", "认证", "报表"]);

        let orders: Vec<i64> = {
            let mut stmt = conn
                .prepare(
                    "SELECT sort_order FROM module WHERE project_id = ?1 ORDER BY sort_order, id",
                )
                .unwrap();
            stmt.query_map(params![project_id], |row| row.get(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        assert_eq!(orders, vec![0, 1, 2], "sort orders are rewritten to contiguous values");

        // New modules still land at the end and cannot collide with existing sort orders
        let new_module = create_module(&conn, project_id, "通知").unwrap();
        let modules = list_modules(&conn, project_id).unwrap();
        assert_eq!(
            modules.last().map(|m| m.id),
            Some(new_module.id),
            "the new module is appended last"
        );
    }

    /// Modules can only be reordered among modules in the same project; cross-project or
    /// missing ids error, and the order stays unchanged.
    #[test]
    fn move_module_rejects_foreign_target() {
        let conn = conn();
        let project_a = create_project(&conn, "A").unwrap().id.unwrap();
        let project_b = create_project(&conn, "B").unwrap().id.unwrap();
        let auth = create_module(&conn, project_a, "认证").unwrap();
        let billing = create_module(&conn, project_a, "计费").unwrap();
        let outsider = create_module(&conn, project_b, "别的项目").unwrap();

        assert!(move_module(&conn, auth.id, Some(outsider.id)).is_err());
        assert!(move_module(&conn, auth.id, Some(999)).is_err());
        assert!(move_module(&conn, 999, Some(billing.id)).is_err());

        let modules = list_modules(&conn, project_a).unwrap();
        assert_eq!(
            modules.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
            vec!["认证", "计费"]
        );
    }

    #[test]
    fn renames_all_three_levels() {
        let conn = conn();
        let project = create_project(&conn, "knot-tr").unwrap();
        let project_id = project.id.unwrap();
        let module = create_module(&conn, project_id, "认证").unwrap();
        let submodule = create_submodule(&conn, module.id, "token 刷新").unwrap();

        let renamed_project = rename_project(&conn, project_id, "  knot  ").unwrap();
        assert_eq!(renamed_project.name, "knot");
        assert_eq!(renamed_project.id, Some(project_id), "the returned value is still the same project");

        let renamed_module = rename_module(&conn, module.id, "鉴权").unwrap();
        assert_eq!(renamed_module.name, "鉴权");
        assert_eq!(
            renamed_module.submodules.len(),
            1,
            "renaming a module must return its existing submodules too"
        );

        let renamed_submodule = rename_submodule(&conn, submodule.id, "刷新令牌").unwrap();
        assert_eq!(renamed_submodule.name, "刷新令牌");

        // The change really lands in the database
        let modules = list_modules(&conn, project_id).unwrap();
        assert_eq!(modules[0].name, "鉴权");
        assert_eq!(modules[0].submodules[0].name, "刷新令牌");
        let projects = list_projects(&conn).unwrap();
        assert_eq!(projects[1].name, "knot");
    }

    /// Blank names are rejected, and the old names in the database are unaffected.
    #[test]
    fn rename_rejects_blank_for_every_level() {
        let conn = conn();
        let project_id = create_project(&conn, "knot-tr").unwrap().id.unwrap();
        let module = create_module(&conn, project_id, "认证").unwrap();
        let submodule = create_submodule(&conn, module.id, "token 刷新").unwrap();

        assert!(rename_project(&conn, project_id, "  ").is_err());
        assert!(rename_module(&conn, module.id, "").is_err());
        assert!(rename_submodule(&conn, submodule.id, "\t").is_err());

        assert_eq!(load_project(&conn, project_id).unwrap().name, "knot-tr");
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

    /// Renaming structure rows must not touch the tasks inside.
    #[test]
    fn rename_does_not_touch_tasks() {
        let conn = conn();
        let project_id = create_project(&conn, "knot-tr").unwrap().id.unwrap();
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

        rename_project(&conn, project_id, "knot").unwrap();
        rename_module(&conn, module.id, "鉴权").unwrap();

        let after = load_task(&conn, task.id).unwrap();
        assert_eq!(after.module_id, Some(module.id));
        assert_eq!(after.title, "登录限流");
    }

    /// A submodule requires a module; it cannot be given alone.
    #[test]
    fn rejects_submodule_without_module() {
        let conn = conn();
        let project_id = create_project(&conn, "knot-tr").unwrap().id.unwrap();
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
    // The editable trio
    // -----------------------------------------------------------------------

    /// Insert a subtask directly into the database: used when a test needs an arbitrary
    /// initial state; the normal path is `create_child`.
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

    /// Create one level of subtask through the real command (the parent defaults to
    /// `todo`).
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

    /// Blank names are rejected and the original stays in the database—so restoring the
    /// original on the frontend cannot drift into "it was already cleared".
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
        let parent = seed_task(&conn); // default mid
        let child = create_child(&conn, &parent, "子任务").task; // default mid

        assert_error_key(
            set_priority(&conn, child.id, Priority::High),
            MessageKey::SubtaskPriorityAboveParent,
        );
        assert_eq!(load_task(&conn, child.id).unwrap().priority.as_str(), "mid");

        // Same level and lower are both allowed.
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
            "a low parent must not produce mid/high subtasks"
        );

        set_priority(&conn, parent.id, Priority::High).unwrap();
        let another = create_child(&conn, &parent, "高父的子任务").task;
        assert_eq!(another.priority.as_str(), "mid", "the default level is still mid");
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

    /// With no subtasks there is nothing to confirm.
    #[test]
    fn setting_done_without_children_needs_no_confirm() {
        let conn = conn();
        let task = seed_task(&conn);

        let plan = plan_set_status(&conn, task.id, Status::Done).unwrap();
        assert!(!plan.needs_confirm);
        assert!(plan.reason.is_none());
        assert_eq!(plan.affects.len(), 1);
    }

    /// Compute first, write later: `plan` must not touch the database.
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
        insert_child(&conn, &task, "补过期用例", Status::Done); // completed ones do not count

        let plan = plan_set_status(&conn, task.id, Status::Done).unwrap();
        assert!(plan.needs_confirm);
        assert_eq!(plan.affects.len(), 2, "only the unfinished one is counted");
        let reason = plan.reason.expect("needs a confirmation reason");
        assert_eq!(reason.key, MessageKey::CascadeDone);
        assert_eq!(
            reason.params.get("count"),
            Some(&MessageParam::Number(1))
        );
    }

    /// Reverting needs no confirmation: subtasks do not move.
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
        assert_eq!(outcome.changed.len(), 2, "parent + one unfinished subtask");
        assert!(outcome.notices.is_empty(), "the user's own click does not produce a notice");

        assert_eq!(load_task(&conn, task.id).unwrap().status.as_str(), "done");
        assert_eq!(load_task(&conn, open).unwrap().status.as_str(), "done");
        assert_eq!(load_task(&conn, closed).unwrap().status.as_str(), "done");
    }

    /// When the user chooses "no cascade", not a single subtask moves.
    #[test]
    fn no_cascade_leaves_children_alone() {
        let conn = conn();
        let task = seed_task(&conn);
        let open = insert_child(&conn, &task, "处理并发刷新", Status::Todo);

        let outcome = apply_set_status(&conn, task.id, Status::Done, false).unwrap();
        assert_eq!(outcome.changed.len(), 1);
        assert_eq!(load_task(&conn, open).unwrap().status.as_str(), "todo");
    }

    /// Reverting a parent leaves subtasks untouched (they may be temporarily inconsistent;
    /// this is intentional).
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
    // Subtasks (one level) and parent status aggregation
    // -----------------------------------------------------------------------

    /// A subtask inherits the parent's project; the two classification levels are always
    /// NULL (ignored even if passed).
    #[test]
    fn subtask_inherits_project_and_nulls_classification() {
        let conn = conn();
        let project_id = create_project(&conn, "knot-tr").unwrap().id.unwrap();
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
        assert_eq!(child.project_id, Some(project_id), "follows the parent's project");
        assert_eq!(child.module_id, None, "placement truth lives on the parent; no redundant classification");
        assert_eq!(child.submodule_id, None);
        assert_eq!(child.status.as_str(), "todo");
        assert_eq!(child.priority.as_str(), "mid");
        assert!(created.notices.is_empty(), "the parent was already unfinished");
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

    /// Only one level is allowed: a subtask cannot have its own subtask.
    #[test]
    fn subtask_rejects_grandchild() {
        let conn = conn();
        let parent = seed_task(&conn);
        let child = create_child(&conn, &parent, "子任务").task;

        assert_error_key(
            create_task(&conn, Container::Todo, None, None, Some(child.id), "孙任务"),
            MessageKey::SubtaskCannotNest,
        );
        assert_eq!(list_tasks(&conn, Container::Todo).unwrap().len(), 2);
    }

    #[test]
    fn subtask_rejects_unknown_parent() {
        let conn = conn();
        assert_not_found(
            create_task(&conn, Container::Todo, None, None, Some(999), "子任务"),
            "task",
            999,
        );
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

        assert_error_key(
            create_task(
                &conn,
                Container::Project { id: b },
                None,
                None,
                Some(parent.id),
                "子任务",
            ),
            MessageKey::SubtaskContainerMismatch,
        );
        assert_eq!(
            list_tasks(&conn, Container::Project { id: b })
                .unwrap()
                .len(),
            0
        );
    }

    /// Sort order is queued per parent and does not mix with top-level tasks or other
    /// parents' subtasks.
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

    /// Event 1: adding an unfinished subtask under a completed parent → the parent goes
    /// back to unfinished.
    #[test]
    fn creating_subtask_on_done_parent_resets_it_to_todo_with_notice() {
        let conn = conn();
        let parent = seed_task(&conn);
        apply_set_status(&conn, parent.id, Status::Done, false).unwrap();

        let created = create_child(&conn, &parent, "新子任务");

        assert_eq!(created.notices.len(), 1);
        assert_eq!(created.notices[0].key, MessageKey::ParentRestoredTodo);
        assert_eq!(created.task.status.as_str(), "todo");
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "todo");
    }

    #[test]
    fn creating_subtask_on_open_parent_changes_nothing() {
        let conn = conn();
        let parent = seed_task(&conn);

        let created = create_child(&conn, &parent, "子任务");

        assert!(created.notices.is_empty());
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "todo");
    }

    /// Event 2: the last unfinished subtask completes → the parent completion is proposed,
    /// not written; confirming it completes the parent.
    #[test]
    fn last_child_done_proposes_parent_done() {
        let conn = conn();
        let parent = seed_task(&conn);
        let first = create_child(&conn, &parent, "子任务一").task;
        let second = create_child(&conn, &parent, "子任务二").task;

        let outcome = apply_set_status(&conn, first.id, Status::Done, false).unwrap();
        assert_eq!(outcome.notices.len(), 1);
        assert_eq!(outcome.notices[0].key, MessageKey::ParentAutoDoing);
        assert_eq!(
            load_task(&conn, parent.id).unwrap().status.as_str(),
            "doing"
        );

        let outcome = apply_set_status(&conn, second.id, Status::Done, false).unwrap();
        assert!(outcome.notices.is_empty(), "completion waits for confirmation");
        assert_eq!(
            load_task(&conn, parent.id).unwrap().status.as_str(),
            "doing",
            "a proposal must not write the parent"
        );
        assert!(outcome.changed.iter().all(|task| task.id != parent.id));

        let proposal = outcome
            .parent_proposals
            .first()
            .expect("the last child should propose completing the parent");
        assert_eq!(proposal.parent_task_id, parent.id);
        assert_eq!(proposal.message.key, MessageKey::ConfirmParentAutoDone);
        confirm_proposal(&conn, &proposal);
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "done");
    }

    /// When a non-last subtask completes, the parent does not complete early but moves from
    /// todo to doing.
    #[test]
    fn non_last_child_done_marks_parent_doing_with_notice() {
        let conn = conn();
        let parent = seed_task(&conn);
        let first = create_child(&conn, &parent, "子任务一").task;
        create_child(&conn, &parent, "子任务二");

        let outcome = apply_set_status(&conn, first.id, Status::Done, false).unwrap();

        assert_eq!(outcome.notices.len(), 1);
        assert_eq!(outcome.notices[0].key, MessageKey::ParentAutoDoing);
        assert_eq!(
            load_task(&conn, parent.id).unwrap().status.as_str(),
            "doing"
        );
    }

    /// When the first subtask moves to doing, the parent automatically goes from todo to
    /// doing.
    #[test]
    fn child_doing_marks_todo_parent_doing_with_notice() {
        let conn = conn();
        let parent = seed_task(&conn);
        let child = create_child(&conn, &parent, "子任务").task;

        let outcome = apply_set_status(&conn, child.id, Status::Doing, false).unwrap();

        assert_eq!(outcome.notices.len(), 1);
        assert_eq!(outcome.notices[0].key, MessageKey::ParentAutoDoing);
        assert_eq!(
            load_task(&conn, parent.id).unwrap().status.as_str(),
            "doing"
        );
        let aggregated = outcome.changed.last().expect("the automatically changed parent must also be returned");
        assert_eq!(aggregated.id, parent.id);
        assert_eq!(aggregated.status.as_str(), "doing");
    }

    /// Subtask revert: if the parent is done → back to doing; other subtasks do not move.
    #[test]
    fn child_revert_reopens_done_parent_with_notice() {
        let conn = conn();
        let parent = seed_task(&conn);
        let first = create_child(&conn, &parent, "子任务一").task;
        let second = create_child(&conn, &parent, "子任务二").task;
        apply_set_status(&conn, first.id, Status::Done, false).unwrap();
        let completed = apply_set_status(&conn, second.id, Status::Done, false).unwrap();
        confirm_proposal(
            &conn,
            completed
                .parent_proposals
                .first()
                .expect("the last child should propose completion"),
        );
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "done");

        let outcome = apply_set_status(&conn, first.id, Status::Todo, false).unwrap();

        assert_eq!(outcome.notices.len(), 1);
        assert_eq!(outcome.notices[0].key, MessageKey::ParentRestoredDoing);
        assert_eq!(
            load_task(&conn, parent.id).unwrap().status.as_str(),
            "doing"
        );
        assert_eq!(load_task(&conn, second.id).unwrap().status.as_str(), "done");
    }

    #[test]
    fn child_reopen_with_all_todo_children_resets_parent_to_todo() {
        let conn = conn();
        let parent = seed_task(&conn);
        let first = create_child(&conn, &parent, "子任务一").task;
        create_child(&conn, &parent, "子任务二");
        apply_set_status(&conn, first.id, Status::Done, false).unwrap();
        assert_eq!(
            load_task(&conn, parent.id).unwrap().status.as_str(),
            "doing"
        );

        let outcome = apply_set_status(&conn, first.id, Status::Todo, false).unwrap();

        assert_eq!(outcome.notices.len(), 1);
        assert_eq!(outcome.notices[0].key, MessageKey::ParentAutoTodo);
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "todo");
    }

    /// Manual parent revert: subtasks do not move and no notice is produced.
    #[test]
    fn manual_parent_revert_leaves_children_and_has_no_notice() {
        let conn = conn();
        let parent = seed_task(&conn);
        let first = create_child(&conn, &parent, "子任务一").task;
        let second = create_child(&conn, &parent, "子任务二").task;
        apply_set_status(&conn, first.id, Status::Done, false).unwrap();
        let completed = apply_set_status(&conn, second.id, Status::Done, false).unwrap();
        confirm_proposal(
            &conn,
            completed
                .parent_proposals
                .first()
                .expect("the last child should propose completion"),
        );
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "done");

        let outcome = apply_set_status(&conn, parent.id, Status::Todo, false).unwrap();

        assert!(outcome.notices.is_empty(), "the user's own click does not produce a notice");
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "todo");
        assert_eq!(load_task(&conn, first.id).unwrap().status.as_str(), "done");
        assert_eq!(load_task(&conn, second.id).unwrap().status.as_str(), "done");
    }

    /// Aggregation is a literal rule: with no subtasks `all()` is true, so completing the
    /// parent is proposed (but not written) and the user can confirm it.
    #[test]
    fn aggregating_empty_child_set_proposes_parent_done() {
        let conn = conn();
        let parent = seed_task(&conn);

        let change = aggregate_parent_status(&conn, parent.id)
            .unwrap()
            .expect("an empty subtask set should trigger aggregation");

        match change {
            ParentChange::Proposed(proposal) => {
                assert_eq!(proposal.parent_task_id, parent.id);
                assert_eq!(proposal.parent_title, parent.title);
                assert_eq!(proposal.status.as_str(), "done");
                assert_eq!(proposal.message.key, MessageKey::ConfirmParentAutoDone);
            }
            other => panic!("expected a completion proposal, got {other:?}"),
        }
        assert_eq!(
            load_task(&conn, parent.id).unwrap().status.as_str(),
            "todo",
            "a proposal must not write the parent"
        );

        apply_set_status(&conn, parent.id, Status::Done, false).unwrap();
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "done");
    }

    // -----------------------------------------------------------------------
    // Delete
    // -----------------------------------------------------------------------

    /// Count first, delete later: the count includes subtasks, and plan itself does not
    /// write.
    #[test]
    fn delete_task_counts_children_then_cascades() {
        let conn = conn();
        let task = seed_task(&conn);
        let open = insert_child(&conn, &task, "处理并发刷新", Status::Todo);
        let done = insert_child(&conn, &task, "补过期用例", Status::Done);

        let plan = plan_delete_task(&conn, task.id).unwrap();
        assert_eq!(plan.task_count, 3, "parent + two subtasks");
        assert_eq!(load_task(&conn, task.id).unwrap().title, "实现 token 轮换");

        let outcome = delete_task(&conn, task.id).unwrap();
        assert!(outcome.notices.is_empty(), "deleting a parent only cascades downward, no upward aggregation");
        assert!(load_task(&conn, task.id).is_err());
        assert!(load_task(&conn, open).is_err());
        assert!(load_task(&conn, done).is_err());
    }

    /// Event 3: deleting the last unfinished subtask proposes completing the parent;
    /// confirming it completes the parent.
    #[test]
    fn deleting_last_unfinished_child_proposes_parent_done() {
        let conn = conn();
        let parent = seed_task(&conn);
        let child = create_child(&conn, &parent, "子任务").task;

        let outcome = delete_task(&conn, child.id).unwrap();

        assert!(outcome.notices.is_empty());
        assert!(load_task(&conn, child.id).is_err());
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "todo");
        let proposal = outcome
            .parent_proposals
            .first()
            .expect("deleting the last unfinished child should propose completion");
        confirm_proposal(&conn, proposal);
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

    /// The parent is already done, and a completed subtask is deleted: status unchanged,
    /// no repeated notice.
    #[test]
    fn deleting_done_child_from_done_parent_emits_no_notice() {
        let conn = conn();
        let parent = seed_task(&conn);
        let child = create_child(&conn, &parent, "子任务").task;
        let completed = apply_set_status(&conn, child.id, Status::Done, false).unwrap();
        confirm_proposal(
            &conn,
            completed
                .parent_proposals
                .first()
                .expect("the last child should propose completion"),
        );
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "done");

        let outcome = delete_task(&conn, child.id).unwrap();

        assert!(outcome.notices.is_empty());
        assert!(outcome.parent_proposals.is_empty());
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "done");
    }

    /// Delete a module: its submodules, tasks directly in the module, tasks in its
    /// submodules, and their subtasks all disappear; default tasks and other projects are
    /// unaffected.
    #[test]
    fn delete_module_counts_submodule_tasks_and_cascades() {
        let conn = conn();
        let project_id = create_project(&conn, "knot-tr").unwrap().id.unwrap();
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
        assert_eq!(plan.task_count, 3, "module task + submodule task + subtask");
        assert_eq!(
            list_tasks(&conn, container).unwrap().len(),
            4,
            "plan does not write"
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
            "submodules are deleted along with the module"
        );
        assert!(load_task(&conn, in_module.id).is_err());
        assert!(load_task(&conn, in_submodule.id).is_err());
        assert!(load_task(&conn, child).is_err());
        assert_eq!(load_task(&conn, outside.id).unwrap().title, "整理文档");
    }

    /// Deleting a submodule only removes the tasks inside it; tasks directly in the module
    /// remain.
    #[test]
    fn delete_submodule_leaves_module_tasks_alone() {
        let conn = conn();
        let project_id = create_project(&conn, "knot-tr").unwrap().id.unwrap();
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

    /// Delete a project: all tasks in it (default tasks and subtasks included) and the
    /// whole module tree disappear; "My Todos" and other projects are untouched.
    #[test]
    fn delete_project_counts_all_tasks_and_cascades() {
        let conn = conn();
        let doomed = create_project(&conn, "knot-tr").unwrap().id.unwrap();
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
        assert_eq!(plan.task_count, 4, "includes default tasks and subtasks");
        assert_eq!(
            list_tasks(&conn, container).unwrap().len(),
            4,
            "plan does not write"
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
        assert!(projects[0].is_todo);
        assert_eq!(projects[0].name, "", "the todo container has no stored name");
        assert_eq!(projects[1].name, "支付网关");
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
    // Notes
    // -----------------------------------------------------------------------

    /// Notes allow newlines and consecutive spaces and stay as-is in the database—collapsing
    /// the summary to one line is a presentation concern.
    #[test]
    fn set_note_keeps_multiline_text() {
        let conn = conn();
        let task = seed_task(&conn);
        let note = "第一行\n第二行\n\n缩进  空格";

        let updated = set_note(&conn, task.id, note).unwrap();
        assert_eq!(updated.note, note);
        assert_eq!(load_task(&conn, task.id).unwrap().note, note);
    }

    /// An empty string means "clear", not "reject".
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
    // Start / end times: maintained automatically by status changes
    // -----------------------------------------------------------------------

    /// The time format is fixed at 16 local-time characters; returning a reference makes
    /// ordering comparisons convenient.
    fn assert_local_time(value: &Option<String>) -> &str {
        let raw = value.as_deref().expect("a status change should record the time automatically");
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

    /// Entering doing: record the start time and leave the end time empty.
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

    /// Doing → done: keep the start time, record the end time, and end is not earlier than
    /// start.
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
            "end {end} must not be earlier than start {:?}",
            start
        );
    }

    /// Back to todo: clear both, and the next round starts fresh.
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

        // Entering doing again is a brand-new round.
        let again = apply_set_status(&conn, task.id, Status::Doing, false)
            .unwrap()
            .changed[0]
            .clone();
        assert_local_time(&again.start_at);
        assert_eq!(again.end_at, None);
    }

    /// Rewriting the same status does not overwrite existing times.
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

    /// Cascade completion: subtasks that never started only get an end; subtasks in doing
    /// keep their own start time.
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

    /// All subtasks completing triggers automatic parent completion, and the parent must
    /// also get an end time.
    #[test]
    fn aggregate_done_stamps_parent_end() {
        let conn = conn();
        let parent = seed_task(&conn);
        let child = create_child(&conn, &parent, "子任务").task;

        let completed = apply_set_status(&conn, child.id, Status::Done, false).unwrap();
        confirm_proposal(
            &conn,
            completed
                .parent_proposals
                .first()
                .expect("the last child should propose completion"),
        );

        let parent_after = load_task(&conn, parent.id).unwrap();
        assert_eq!(parent_after.status.as_str(), "done");
        assert_local_time(&parent_after.end_at);
    }

    /// A subtask revert leaves all subtasks unfinished: the parent returns to todo and both
    /// times are cleared.
    #[test]
    fn aggregate_reopen_resets_parent_to_todo_and_clears_times() {
        let conn = conn();
        let parent = seed_task(&conn);
        let child = create_child(&conn, &parent, "子任务").task;

        apply_set_status(&conn, parent.id, Status::Doing, false).unwrap();
        assert_local_time(&load_task(&conn, parent.id).unwrap().start_at);
        let completed = apply_set_status(&conn, child.id, Status::Done, false).unwrap();
        confirm_proposal(
            &conn,
            completed
                .parent_proposals
                .first()
                .expect("the last child should propose completion"),
        );
        assert_local_time(&load_task(&conn, parent.id).unwrap().end_at);

        let outcome = apply_set_status(&conn, child.id, Status::Todo, false).unwrap();

        assert_eq!(outcome.notices[0].key, MessageKey::ParentRestoredTodo);
        let reopened = load_task(&conn, parent.id).unwrap();
        assert_eq!(reopened.status.as_str(), "todo");
        assert_eq!(reopened.start_at, None);
        assert_eq!(reopened.end_at, None);
    }

    // -----------------------------------------------------------------------
    // Classification moves
    // -----------------------------------------------------------------------

    /// After moving, the task lands at the end of the target group: the sort order is the
    /// target group's existing MAX + 1, not the old value.
    #[test]
    fn move_task_to_module_appends_in_target_group() {
        let conn = conn();
        let project_id = create_project(&conn, "knot-tr").unwrap().id.unwrap();
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
        assert_eq!(task.sort_order, 0, "the default group starts counting from zero");
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

    /// Cross-project move: the parent moves to B's module, and subtasks only follow the
    /// container (classification is still expressed by the parent).
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
        assert_eq!(child_after.project_id, Some(b), "subtasks follow the parent to the new container");
        assert_eq!(child_after.module_id, None);
        assert_eq!(child_after.submodule_id, None);
    }

    /// Moving back to "My Todos" clears all three classification levels; subtask containers
    /// are cleared too.
    #[test]
    fn move_task_to_todo_clears_classification() {
        let conn = conn();
        let project_id = create_project(&conn, "knot-tr").unwrap().id.unwrap();
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

    /// Invalid targets are all rejected, and not a single field of the moved task changes.
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
            // module from another project
            MoveTarget {
                container: Container::Project { id: a },
                module_id: Some(module_b.id),
                submodule_id: None,
            },
            // submodule without a module
            MoveTarget {
                container: Container::Project { id: a },
                module_id: None,
                submodule_id: Some(submodule_a),
            },
            // mismatched submodule
            MoveTarget {
                container: Container::Project { id: a },
                module_id: Some(module_a.id),
                submodule_id: Some(submodule_b),
            },
            // "My Todos" with a module
            MoveTarget {
                container: Container::Todo,
                module_id: Some(module_a.id),
                submodule_id: None,
            },
            // unknown project
            MoveTarget {
                container: Container::Project { id: 999 },
                module_id: None,
                submodule_id: None,
            },
        ];

        for target in attempts {
            assert!(
                move_task(&conn, task.id, target).is_err(),
                "{target:?} should be rejected"
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

    /// Subtasks cannot be moved on their own: placement truth lives on the parent.
    #[test]
    fn move_task_rejects_subtask() {
        let conn = conn();
        let target_project = create_project(&conn, "B").unwrap().id.unwrap();
        let parent = seed_task(&conn);
        let child = insert_child(&conn, &parent, "子任务", Status::Todo);

        assert_error_key(
            move_task(
                &conn,
                child,
                MoveTarget {
                    container: Container::Project { id: target_project },
                    module_id: None,
                    submodule_id: None,
                },
            ),
            MessageKey::SubtaskCannotMove,
        );
        assert_eq!(load_task(&conn, parent.id).unwrap().project_id, None);
        assert_eq!(load_task(&conn, child).unwrap().project_id, None);
    }

    // -----------------------------------------------------------------------
    // Parent-child conversion
    // -----------------------------------------------------------------------

    /// After demotion, placement follows the new parent, the two classification levels are
    /// cleared, and the task appends to the target's subtasks.
    #[test]
    fn demote_appends_under_target_and_clears_classification() {
        let conn = conn();
        let project_id = create_project(&conn, "knot-tr").unwrap().id.unwrap();
        let container = Container::Project { id: project_id };
        let module = create_module(&conn, project_id, "认证").unwrap();
        let target = create_task(&conn, container, Some(module.id), None, None, "目标父任务")
            .unwrap()
            .task;
        let task = create_task(&conn, container, None, None, None, "待降级")
            .unwrap()
            .task;
        assert_eq!(task.sort_order, 0);

        let outcome = reparent_task(&conn, task.id, Some(target.id)).unwrap();

        assert_eq!(outcome.task.parent_task_id, Some(target.id));
        assert_eq!(outcome.task.project_id, Some(project_id));
        assert_eq!(outcome.task.module_id, None, "placement is now expressed by the parent");
        assert_eq!(outcome.task.submodule_id, None);
        assert_eq!(outcome.task.sort_order, 0, "the target has an empty subtask list, so it goes first");
        assert!(outcome.notices.is_empty(), "the target was already unfinished");

        let after = load_task(&conn, task.id).unwrap();
        assert_eq!(after.parent_task_id, Some(target.id));
        assert_eq!(after.module_id, None);
    }

    /// A subtask cannot outrank its new parent; demotion clamps it to the parent's level.
    #[test]
    fn demote_clamps_priority_to_target() {
        let conn = conn();
        let target = seed_task(&conn);
        let task = seed_task(&conn);
        set_priority(&conn, target.id, Priority::Low).unwrap();
        set_priority(&conn, task.id, Priority::High).unwrap();

        let outcome = reparent_task(&conn, task.id, Some(target.id)).unwrap();

        assert_eq!(outcome.task.priority.as_str(), "low");
        assert_eq!(load_task(&conn, task.id).unwrap().priority.as_str(), "low");
    }

    /// A parent with subtasks cannot be demoted directly, or grandchild tasks would appear.
    #[test]
    fn demote_rejects_parent_with_children() {
        let conn = conn();
        let target = seed_task(&conn);
        let parent = seed_task(&conn);
        let child = create_child(&conn, &parent, "子任务").task;

        assert_error_key(
            reparent_task(&conn, parent.id, Some(target.id)),
            MessageKey::HasChildrenCannotDemote,
        );
        assert_eq!(load_task(&conn, parent.id).unwrap().parent_task_id, None);
        assert_eq!(
            load_task(&conn, child.id).unwrap().parent_task_id,
            Some(parent.id)
        );
    }

    /// Self, subtasks, cross-container, and unknown targets are all rejected, and not a
    /// single field of the original task changes.
    #[test]
    fn demote_rejects_invalid_targets() {
        let conn = conn();
        let a = create_project(&conn, "A").unwrap().id.unwrap();
        let b = create_project(&conn, "B").unwrap().id.unwrap();
        let target_a = create_task(
            &conn,
            Container::Project { id: a },
            None,
            None,
            None,
            "A 的父任务",
        )
        .unwrap()
        .task;
        let target_b = create_task(
            &conn,
            Container::Project { id: b },
            None,
            None,
            None,
            "B 的父任务",
        )
        .unwrap()
        .task;
        let child = create_child(&conn, &target_a, "A 的子任务").task;
        let task = create_task(
            &conn,
            Container::Project { id: a },
            None,
            None,
            None,
            "待降级",
        )
        .unwrap()
        .task;

        for (target_id, expected) in [
            (task.id, MessageKey::CannotDemoteToSelf),
            (child.id, MessageKey::SubtaskCannotNest),
            (target_b.id, MessageKey::SubtaskParentContainerMismatch),
            (999, MessageKey::NotFound),
        ] {
            let error = reparent_task(&conn, task.id, Some(target_id)).unwrap_err();
            assert_eq!(error.key, expected, "{target_id} should be rejected: {error}");
            let after = load_task(&conn, task.id).unwrap();
            assert_eq!(after.parent_task_id, None);
            assert_eq!(after.project_id, Some(a));
        }
    }

    /// Demotion recomputes the target parent: a subtask in doing pushes a todo parent to
    /// doing.
    #[test]
    fn demote_updates_target_status_with_notice() {
        let conn = conn();
        let target = seed_task(&conn);
        let task = create_task(&conn, Container::Todo, None, None, None, "进行中的任务")
            .unwrap()
            .task;
        apply_set_status(&conn, task.id, Status::Doing, false).unwrap();

        let outcome = reparent_task(&conn, task.id, Some(target.id)).unwrap();

        assert_eq!(
            load_task(&conn, target.id).unwrap().status.as_str(),
            "doing"
        );
        assert_eq!(outcome.notices[0].key, MessageKey::ParentAutoDoing);
    }

    /// Promotion inherits the old parent's module / submodule and appends to the end of
    /// that top-level group.
    #[test]
    fn promote_inherits_parent_placement_and_appends() {
        let conn = conn();
        let project_id = create_project(&conn, "knot-tr").unwrap().id.unwrap();
        let container = Container::Project { id: project_id };
        let module = create_module(&conn, project_id, "认证").unwrap();
        let submodule = create_submodule(&conn, module.id, "token 刷新").unwrap();
        let parent = create_task(
            &conn,
            container,
            Some(module.id),
            Some(submodule.id),
            None,
            "原父任务",
        )
        .unwrap()
        .task;
        let sibling = create_task(
            &conn,
            container,
            Some(module.id),
            Some(submodule.id),
            None,
            "同组已有任务",
        )
        .unwrap()
        .task;
        let child = create_child(&conn, &parent, "要升级的子任务").task;

        let outcome = reparent_task(&conn, child.id, None).unwrap();

        assert_eq!(outcome.task.parent_task_id, None);
        assert_eq!(outcome.task.project_id, Some(project_id));
        assert_eq!(outcome.task.module_id, Some(module.id));
        assert_eq!(outcome.task.submodule_id, Some(submodule.id));
        assert_eq!(
            outcome.task.sort_order,
            sibling.sort_order + 1,
            "appended at the end of the original group"
        );
    }

    /// After the only subtask is promoted away, the old parent completion is proposed under
    /// the zero-subtask rule; confirming it completes the parent.
    #[test]
    fn promote_only_child_proposes_old_parent_done() {
        let conn = conn();
        let parent = seed_task(&conn);
        let child = create_child(&conn, &parent, "唯一子任务").task;

        let outcome = reparent_task(&conn, child.id, None).unwrap();

        assert!(outcome.notices.is_empty());
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "todo");
        let proposal = outcome
            .parent_proposals
            .first()
            .expect("promoting the only child should propose completion");
        confirm_proposal(&conn, proposal);
        assert_eq!(load_task(&conn, parent.id).unwrap().status.as_str(), "done");
    }

    /// A subtask can move directly to another top-level task; the old parent is
    /// re-aggregated after the child leaves.
    #[test]
    fn subtask_can_move_to_another_parent() {
        let conn = conn();
        let old_parent = seed_task(&conn);
        let new_parent = seed_task(&conn);
        let child = create_child(&conn, &old_parent, "child").task;

        let outcome = reparent_task(&conn, child.id, Some(new_parent.id)).unwrap();

        assert_eq!(outcome.task.parent_task_id, Some(new_parent.id));
        assert_eq!(
            load_task(&conn, child.id).unwrap().parent_task_id,
            Some(new_parent.id)
        );
        assert_eq!(outcome.parent_proposals.len(), 1);
        assert_eq!(outcome.parent_proposals[0].parent_task_id, old_parent.id);
        assert_eq!(
            load_task(&conn, old_parent.id).unwrap().status.as_str(),
            "todo",
            "the proposal is not written until confirmed"
        );
        assert_eq!(
            load_task(&conn, new_parent.id).unwrap().status.as_str(),
            "todo"
        );

        confirm_proposal(&conn, &outcome.parent_proposals[0]);
        assert_eq!(
            load_task(&conn, old_parent.id).unwrap().status.as_str(),
            "done"
        );
    }

    /// Moving a done subtask can make both the old and the new parent eligible for
    /// completion, so both proposals are returned.
    #[test]
    fn retargeting_a_subtask_can_propose_both_parents() {
        let conn = conn();
        let old_parent = seed_task(&conn);
        let new_parent = seed_task(&conn);
        let leaving = create_child(&conn, &old_parent, "leaving").task;
        let remaining = create_child(&conn, &old_parent, "remaining").task;
        apply_set_status(&conn, leaving.id, Status::Done, false).unwrap();
        let old_done = apply_set_status(&conn, remaining.id, Status::Done, false).unwrap();
        assert_eq!(old_done.parent_proposals[0].parent_task_id, old_parent.id);

        let incoming = create_child(&conn, &new_parent, "incoming").task;
        let new_done = apply_set_status(&conn, incoming.id, Status::Done, false).unwrap();
        assert_eq!(new_done.parent_proposals[0].parent_task_id, new_parent.id);

        let outcome = reparent_task(&conn, leaving.id, Some(new_parent.id)).unwrap();

        let proposed: Vec<i64> = outcome
            .parent_proposals
            .iter()
            .map(|proposal| proposal.parent_task_id)
            .collect();
        assert_eq!(proposed, vec![old_parent.id, new_parent.id]);
        assert_eq!(
            load_task(&conn, old_parent.id).unwrap().status.as_str(),
            "doing"
        );
        assert_eq!(
            load_task(&conn, new_parent.id).unwrap().status.as_str(),
            "todo"
        );

        for proposal in &outcome.parent_proposals {
            confirm_proposal(&conn, proposal);
        }
        assert_eq!(
            load_task(&conn, old_parent.id).unwrap().status.as_str(),
            "done"
        );
        assert_eq!(
            load_task(&conn, new_parent.id).unwrap().status.as_str(),
            "done"
        );
    }

    /// Promotion / demotion changes only the hierarchy, not the task's own status or times.
    #[test]
    fn reparent_keeps_task_status_and_times() {
        let conn = conn();
        let target = seed_task(&conn);
        let task = create_task(&conn, Container::Todo, None, None, None, "进行中的任务")
            .unwrap()
            .task;
        let doing = apply_set_status(&conn, task.id, Status::Doing, false).unwrap();
        let start_at = doing.changed[0].start_at.clone();

        reparent_task(&conn, task.id, Some(target.id)).unwrap();
        let demoted = load_task(&conn, task.id).unwrap();
        assert_eq!(demoted.status.as_str(), "doing");
        assert_eq!(demoted.start_at, start_at);
        assert_eq!(demoted.end_at, None);

        reparent_task(&conn, task.id, None).unwrap();
        let promoted = load_task(&conn, task.id).unwrap();
        assert_eq!(promoted.status.as_str(), "doing");
        assert_eq!(promoted.start_at, start_at);
        assert_eq!(promoted.end_at, None);
    }

    #[test]
    fn promote_rejects_top_level_task() {
        let conn = conn();
        let task = seed_task(&conn);

        assert_error_key(
            reparent_task(&conn, task.id, None),
            MessageKey::AlreadyTopLevel,
        );
        assert_eq!(load_task(&conn, task.id).unwrap().parent_task_id, None);
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
    // IPC field-name contract (frontend api.ts depends on these names; change both sides
    // together)
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
            assert!(json.get(key).is_some(), "missing {key}");
        }
        assert_eq!(json["status"], "todo");
        assert_eq!(json["priority"], "mid");
        assert!(json["startAt"].is_null(), "unset times are null, not a missing field");
        assert!(!json["createdAt"].is_null(), "a new task should have a creation time");
    }

    #[test]
    fn status_plan_wire_shape_is_camel_case() {
        let json = serde_json::to_value(StatusPlan {
            affects: Vec::new(),
            needs_confirm: true,
            reason: Some(Message::new(MessageKey::CascadeDone).param("count", 1i64)),
        })
        .unwrap();

        assert_eq!(json["needsConfirm"], true);
        assert_eq!(json["reason"]["key"], "confirm.cascadeDone");
        assert_eq!(json["reason"]["params"]["count"], 1);
        assert!(json.get("needs_confirm").is_none(), "must not be snake_case");
    }

    #[test]
    fn delete_plan_wire_shape_is_camel_case() {
        let json = serde_json::to_value(DeletePlan { task_count: 3 }).unwrap();
        assert_eq!(json["taskCount"], 3);
        assert!(json.get("task_count").is_none(), "must not be snake_case");
    }

    #[test]
    fn task_created_wire_shape_is_camel_case() {
        let conn = conn();
        let json = serde_json::to_value(TaskCreated {
            task: seed_task(&conn),
            notices: vec![Message::new(MessageKey::ParentAutoDoing)],
        })
        .unwrap();

        assert_eq!(json["notices"][0]["key"], "notice.parentAutoDoing");
        assert!(
            json["notices"][0].get("params").is_none(),
            "empty params are omitted"
        );
        assert!(json["task"].get("parentTaskId").is_some());
        assert!(
            json["task"].get("parent_task_id").is_none(),
            "must not be snake_case"
        );
    }

    #[test]
    fn delete_outcome_wire_shape_is_camel_case() {
        let json = serde_json::to_value(DeleteOutcome {
            notices: vec![Message::new(MessageKey::ParentAutoDoing)],
            parent_proposals: Vec::new(),
        })
        .unwrap();
        assert_eq!(json["notices"][0]["key"], "notice.parentAutoDoing");
        assert!(json["parentProposals"].as_array().unwrap().is_empty());
    }

    #[test]
    fn parent_proposal_wire_shape_is_camel_case() {
        let json = serde_json::to_value(ParentStatusProposal {
            parent_task_id: 7,
            parent_title: "Parent".to_string(),
            status: Status::Done,
            message: Message::new(MessageKey::ConfirmParentAutoDone)
                .param("title", "Parent"),
        })
        .unwrap();

        assert_eq!(json["parentTaskId"], 7);
        assert_eq!(json["parentTitle"], "Parent");
        assert_eq!(json["status"], "done");
        assert_eq!(json["message"]["key"], "confirm.parentAutoDone");
        assert!(json.get("parent_task_id").is_none(), "must not be snake_case");
    }

    #[test]
    fn status_outcome_wire_shape_is_camel_case() {
        let json = serde_json::to_value(StatusOutcome {
            changed: Vec::new(),
            notices: Vec::new(),
            parent_proposals: Vec::new(),
        })
        .unwrap();
        assert!(json.get("changed").is_some());
        assert!(json.get("notices").is_some());
        assert!(json["parentProposals"].as_array().unwrap().is_empty());
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
